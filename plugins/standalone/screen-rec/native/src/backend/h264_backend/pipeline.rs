use std::path::Path;

use ffmpeg_next::{self as ffmpeg};
use tynd_plugin_sdk::{PluginError, PluginResult};

use crate::Preset;
/// Owns the ffmpeg muxer + encoder + scaler. Built lazily inside the
/// worker thread once the first scap frame has revealed the actual
/// capture resolution (`scap` may scale away from the requested
/// `output_resolution` due to DPI / Retina). Wrapping the inner state
/// in `Option` lets `Drop` reclaim ownership and call `finish()` —
/// which consumes the inner — even when `push_bgra` panics or the
/// thread is killed mid-recording, so the MP4 always gets its `moov`
/// atom and stays playable.
pub(super) struct Pipeline {
    inner: Option<PipelineInner>,
}

struct PipelineInner {
    octx: ffmpeg::format::context::Output,
    encoder: ffmpeg::codec::encoder::video::Encoder,
    scaler: ffmpeg::software::scaling::Context,
    src_width: u32,
    src_height: u32,
    enc_time_base: ffmpeg::Rational,
    stream_time_base: ffmpeg::Rational,
    stream_index: usize,
    pts: i64,
}

impl Pipeline {
    pub(super) fn new(
        output_path: &Path,
        codec_name: &'static str,
        width: u32,
        height: u32,
        fps: u32,
        bitrate_kbps: u32,
        preset: Preset,
    ) -> PluginResult<Self> {
        // 1. Open the output container. ffmpeg picks the muxer from the file extension — `.mp4` /
        //    `.mkv` / `.mov` all work.
        let mut octx = ffmpeg::format::output(&output_path).map_err(|e| {
            PluginError::Custom(format!("screen-rec: open output {:?}: {e}", output_path))
        })?;

        // 2. Resolve the encoder by name. Already validated via `pick_encoder`, but we re-fetch
        //    here to get the typed handle ffmpeg-next needs for `add_stream` / context init.
        let codec = ffmpeg::encoder::find_by_name(codec_name).ok_or_else(|| {
            PluginError::Custom(format!("screen-rec: encoder {codec_name} disappeared"))
        })?;

        let global_header = octx
            .format()
            .flags()
            .contains(ffmpeg::format::Flags::GLOBAL_HEADER);

        // 3. Add the video stream + build the encoder context.
        let mut ost = octx
            .add_stream(codec)
            .map_err(|e| PluginError::Custom(format!("screen-rec: add stream: {e}")))?;
        let stream_index = ost.index();

        let mut encoder = ffmpeg::codec::context::Context::new_with_codec(codec)
            .encoder()
            .video()
            .map_err(|e| PluginError::Custom(format!("screen-rec: encoder ctx: {e}")))?;

        encoder.set_width(width);
        encoder.set_height(height);
        // YUV420P is the universal format every H.264 encoder accepts.
        // Hardware encoders (NVENC, QSV) negotiate NV12 internally if
        // they prefer it; we don't have to guess.
        encoder.set_format(ffmpeg::format::Pixel::YUV420P);
        encoder.set_bit_rate((bitrate_kbps as usize) * 1000);
        encoder.set_max_bit_rate((bitrate_kbps as usize) * 1000);

        let time_base = ffmpeg::Rational(1, fps as i32);
        encoder.set_time_base(time_base);
        encoder.set_frame_rate(Some(ffmpeg::Rational(fps as i32, 1)));
        ost.set_time_base(time_base);

        if global_header {
            encoder.set_flags(ffmpeg::codec::Flags::GLOBAL_HEADER);
        }

        // 4. Forward the preset + sane defaults via the codec Dictionary. Each encoder accepts a
        //    slightly different vocabulary; libx264 / libx265 take the classic `ultrafast..slow`
        //    words, NVENC takes `p1..p7` (we map the closest one), VideoToolbox ignores it.
        let mut dict = ffmpeg::Dictionary::new();
        let preset_str = preset_for(codec_name, preset);
        if let Some(p) = preset_str {
            dict.set("preset", p);
        }
        // Tune for low-latency capture. NVENC accepts `zerolatency`
        // via this key; libx264 takes the classic `tune` option.
        if codec_name.starts_with("h264_nvenc") || codec_name.starts_with("hevc_nvenc") {
            dict.set("tune", "ll");
        } else if codec_name == "libx264" || codec_name == "libx265" {
            dict.set("tune", "zerolatency");
        }

        let opened_encoder = encoder.open_with(dict).map_err(|e| {
            PluginError::Custom(format!("screen-rec: open encoder {codec_name}: {e}"))
        })?;
        ost.set_parameters(&opened_encoder);
        let stream_time_base = ost.time_base();

        // 5. Header — must come *after* parameters are set, otherwise the muxer writes the wrong
        //    codec config + most players can't decode the file.
        octx.write_header()
            .map_err(|e| PluginError::Custom(format!("screen-rec: write header: {e}")))?;

        // 6. Pre-build the BGRA → YUV420P scaler. Source pixel format is whatever `scap` hands us
        //    (BGRA on every platform we support); destination matches the encoder.
        let scaler = ffmpeg::software::scaling::Context::get(
            ffmpeg::format::Pixel::BGRA,
            width,
            height,
            ffmpeg::format::Pixel::YUV420P,
            width,
            height,
            ffmpeg::software::scaling::Flags::BILINEAR,
        )
        .map_err(|e| PluginError::Custom(format!("screen-rec: swscale init: {e}")))?;

        Ok(Self {
            inner: Some(PipelineInner {
                octx,
                encoder: opened_encoder,
                scaler,
                src_width: width,
                src_height: height,
                enc_time_base: time_base,
                stream_time_base,
                stream_index,
                pts: 0,
            }),
        })
    }

    /// Push one BGRA frame through the encoder. The frame is wrapped
    /// in an ffmpeg `Video` frame (BGRA), scaled to YUV420P, then
    /// `send_frame`'d to the encoder. Every packet ffmpeg has ready
    /// is drained immediately so the on-disk file stays close to
    /// real-time.
    pub(super) fn push_bgra(&mut self, data: &[u8], width: u32, height: u32) -> Result<(), String> {
        let inner = self
            .inner
            .as_mut()
            .ok_or_else(|| "pipeline finished".to_string())?;
        // Source frames must match the encoder dimensions — `scap`
        // scales to the requested resolution server-side, so a
        // mismatch here means an unexpected capture target. We log
        // and drop the frame instead of erroring out the whole
        // recording.
        if width != inner.src_width || height != inner.src_height {
            return Err(format!(
                "frame {width}x{height} != encoder {}x{}",
                inner.src_width, inner.src_height
            ));
        }

        // Hard stop before we wrap around to a duplicate PTS — once
        // the encoder sees two frames with the same timestamp it
        // silently drops the second one and most muxers refuse to
        // write a packet at the same DTS as the previous one. Surface
        // the error so the worker loop can flush and exit cleanly
        // instead of pumping discarded frames forever.
        if inner.pts == i64::MAX {
            tracing::warn!(
                target: "tynd::screen_rec",
                "PTS overflow at i64::MAX after ~{} years at {} fps — stopping",
                i64::MAX / (inner.enc_time_base.denominator() as i64).max(1) / 31_536_000,
                inner.enc_time_base.denominator(),
            );
            return Err("pts overflow".to_string());
        }

        // Wrap the BGRA buffer in a Video frame. The frame allocates
        // its own buffer (we can't mmap the `scap` slice into ffmpeg
        // safely — the lifetime ends at the next `get_next_frame`),
        // so we copy row-by-row honouring the linesize ffmpeg picks.
        let mut src = ffmpeg::frame::Video::new(ffmpeg::format::Pixel::BGRA, width, height);
        copy_bgra_into_frame(data, &mut src)?;

        let mut dst = ffmpeg::frame::Video::empty();
        inner
            .scaler
            .run(&src, &mut dst)
            .map_err(|e| format!("swscale: {e}"))?;
        dst.set_pts(Some(inner.pts));
        inner.pts += 1;

        inner
            .encoder
            .send_frame(&dst)
            .map_err(|e| format!("send_frame: {e}"))?;
        Self::drain_packets(inner)
    }

    /// Pull every packet the encoder is ready to emit and write it
    /// to the muxer. Called after each `send_frame` and once more
    /// after `send_eof`.
    fn drain_packets(inner: &mut PipelineInner) -> Result<(), String> {
        // Imported for the explicit EAGAIN match below — see
        // `receive_packet` arm.
        use ffmpeg::util::error::EAGAIN;

        let mut packet = ffmpeg::Packet::empty();
        loop {
            match inner.encoder.receive_packet(&mut packet) {
                Ok(()) => {
                    packet.set_stream(inner.stream_index);
                    packet.rescale_ts(inner.enc_time_base, inner.stream_time_base);
                    packet
                        .write_interleaved(&mut inner.octx)
                        .map_err(|e| format!("mux packet: {e}"))?;
                },
                // EAGAIN — encoder needs more input frames before it
                // can emit a packet. Anything else (ENOSPC,
                // permission denied, ENOMEM, …) is a hard failure
                // that we must surface so the worker can stop the
                // recording instead of silently dropping every frame.
                Err(ffmpeg::Error::Other { errno }) if errno == EAGAIN => return Ok(()),
                Err(ffmpeg::Error::Eof) => return Ok(()),
                Err(e) => return Err(format!("receive_packet: {e}")),
            }
        }
    }

    /// Flush the encoder and write the muxer trailer — without this
    /// the output container has no `moov` atom and most players
    /// refuse to open it. Takes `&mut self` so it can be called from
    /// `Drop` as well as the worker exit path; subsequent calls are
    /// no-ops because the inner state is consumed on the first run.
    pub(super) fn finish(&mut self) -> Result<(), String> {
        let Some(mut inner) = self.inner.take() else {
            return Ok(());
        };
        let flush_result = inner
            .encoder
            .send_eof()
            .map_err(|error| format!("send_eof: {error}"))
            .and_then(|()| Self::drain_packets(&mut inner));
        let trailer_result = inner
            .octx
            .write_trailer()
            .map_err(|error| format!("write_trailer: {error}"));
        super::super::merge_worker_results(flush_result, trailer_result)
    }
}

impl Drop for Pipeline {
    /// RAII safety net — guarantees `write_trailer` runs even if the
    /// worker thread panics mid-`push_bgra`, the host process kills
    /// us, or any other path exits without an explicit `finish()`
    /// call. Without this the MP4 has no `moov` atom and is
    /// unplayable, despite all the encoded frames being on disk.
    /// Errors are swallowed because we're already on a failure path
    /// and there's nowhere useful to surface them.
    fn drop(&mut self) {
        if self.inner.is_some() {
            if let Err(e) = self.finish() {
                tracing::error!(
                    target: "tynd::screen_rec",
                    "finalize encoder during drop: {e}"
                );
            }
        }
    }
}

/// Copy `src` (BGRA, packed) into the ffmpeg frame's plane 0,
/// honouring the linesize ffmpeg chose (which may include padding
/// for SIMD alignment).
fn copy_bgra_into_frame(src: &[u8], dst: &mut ffmpeg::frame::Video) -> Result<(), String> {
    let width = dst.width() as usize;
    let height = dst.height() as usize;
    let src_stride = width * 4;
    if src.len() < src_stride * height {
        return Err(format!(
            "BGRA buffer too small: {} < {} bytes",
            src.len(),
            src_stride * height
        ));
    }
    let dst_stride = dst.stride(0);
    // ffmpeg may pick a `dst_stride` *smaller* than `src_stride` on
    // exotic pixel formats (very narrow widths where libavutil rounds
    // down for SIMD alignment). Indexing past the end of either side
    // would panic and tear down the worker thread, so clamp the copy
    // width to the smaller of the two — the trailing alignment bytes
    // are encoder-side padding and never read.
    let copy_w = src_stride.min(dst_stride);
    let dst_data = dst.data_mut(0);
    for y in 0..height {
        let s = &src[y * src_stride..y * src_stride + copy_w];
        let d = &mut dst_data[y * dst_stride..y * dst_stride + copy_w];
        d.copy_from_slice(s);
    }
    Ok(())
}

/// Map our normalised preset enum onto whatever vocabulary the
/// chosen encoder accepts. Returns `None` for encoders that ignore
/// the preset key (VideoToolbox, VA-API).
fn preset_for(codec_name: &str, preset: Preset) -> Option<&'static str> {
    match codec_name {
        "libx264" | "libx265" => Some(match preset {
            Preset::Ultrafast => "ultrafast",
            Preset::Fast => "fast",
            Preset::Medium => "medium",
            Preset::Slow => "slow",
        }),
        "h264_nvenc" | "hevc_nvenc" => Some(match preset {
            // NVENC presets are p1 (fastest) .. p7 (slowest).
            Preset::Ultrafast => "p1",
            Preset::Fast => "p3",
            Preset::Medium => "p4",
            Preset::Slow => "p6",
        }),
        "h264_qsv" | "hevc_qsv" => Some(match preset {
            // QSV uses the same vocabulary as libx264.
            Preset::Ultrafast => "veryfast",
            Preset::Fast => "fast",
            Preset::Medium => "medium",
            Preset::Slow => "slow",
        }),
        // VideoToolbox / VA-API don't take a `preset` key — they
        // expose their own knobs (`realtime`, `quality`, etc.) that
        // we leave at the encoder defaults for now.
        _ => None,
    }
}
