//! MJPEG encoder backend.
//!
//! ## Why MJPEG (and not H.264 in MP4)?
//!
//! True H.264 encoding from pure Rust requires either bundling Cisco's
//! OpenH264 binary (license headache for downstreams) or depending on
//! `ffmpeg-next` (heavy native build, codec patent surface). MJPEG is
//! intra-only — every frame is a self-contained JPEG — which makes the
//! encoder a one-line call into the `image` crate and produces a file
//! that ffmpeg / mpv / VLC play directly. The trade-off is filesize
//! (5–10× H.264 at the same quality), which is what the
//! `gpu-encode` feature + `h264_backend` solves on builds that opt in.
//!
//! ## Wire format
//!
//! Files are concatenated JPEGs (each starts with `FF D8` and ends
//! with `FF D9`). ffmpeg auto-detects the format with `-f mjpeg`;
//! Quicklook / Finder don't recognise it, so we suggest users
//! re-encode for distribution.

use std::{
    fs::{File, OpenOptions},
    io::{BufWriter, Write},
    sync::atomic::{AtomicBool, Ordering},
};

use tynd_plugin_sdk::{PluginError, PluginResult};

use super::{
    merge_worker_results, spawn_encoder_thread, EncoderThread, FrameSource, NextFrame,
    RecordingOpts, FRAME_POLL,
};

/// Spawn the MJPEG encoder thread. The capture thread owns the
/// `scap::Capturer`; this worker pulls BGRA frames, writes JPEG samples to the
/// output file and flushes it once `running` flips to false.
///
/// The output is opened without truncating so a permission problem still
/// fails fast, but an existing file is only truncated (and a new one only
/// kept) once capture has started: a failed start leaves the filesystem as it
/// was.
pub(super) fn spawn(opts: &RecordingOpts) -> PluginResult<EncoderThread> {
    let existed = std::path::Path::new(&opts.output).exists();
    let file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(&opts.output)
        .map_err(|error| {
            PluginError::Custom(format!(
                "screen-rec: open output {:?}: {error}",
                opts.output
            ))
        })?;
    let writer = BufWriter::new(file);

    let encoder = spawn_encoder_thread(
        "tynd-screen-rec-mjpeg",
        "mjpeg",
        opts,
        move |source, running, paused| run_loop(&source, writer, &running, &paused),
    );
    if encoder.is_err() && !existed {
        let _ = std::fs::remove_file(&opts.output);
    }
    encoder
}
/// Inner encoder loop — runs in its own thread. Pulls frames from
/// the capture thread until `running` flips, encoding each one as JPEG
/// and concatenating into the output file.
///
/// `FrameSource::next_frame` polls with a timeout, so a static screen (no
/// frames delivered) cannot keep this loop from noticing `running == false`
/// and flushing the file.
fn run_loop(
    source: &FrameSource,
    mut writer: BufWriter<File>,
    running: &AtomicBool,
    paused: &AtomicBool,
) -> Result<(), String> {
    let work_result = (|| {
        // The recording starts now: drop any previous content of the file.
        writer
            .get_ref()
            .set_len(0)
            .map_err(|error| format!("truncate output: {error}"))?;
        // Reusable scratch buffers — sized to the first frame, then reused
        // across the recording. Avoids two multi-MB allocations per frame.
        let mut rgb_scratch: Vec<u8> = Vec::new();
        let mut jpeg_scratch: Vec<u8> = Vec::new();
        while running.load(Ordering::SeqCst) {
            let frame = match source.next_frame(FRAME_POLL) {
                NextFrame::Frame(frame) => frame,
                NextFrame::Idle => continue,
                NextFrame::Closed(_) if !running.load(Ordering::Acquire) => break,
                NextFrame::Closed(error) => return Err(error),
            };
            if paused.load(Ordering::SeqCst) {
                continue;
            }
            encode_and_write(&frame, &mut writer, &mut rgb_scratch, &mut jpeg_scratch)?;
        }
        Ok(())
    })();
    let flush_result = writer
        .flush()
        .map_err(|error| format!("flush output: {error}"));
    merge_worker_results(work_result, flush_result)
}
/// Convert one `scap::frame::Frame` (BGRA) to JPEG bytes and
/// append to the output stream. Wide pixel formats are bridged
/// through `image::RgbImage` which the JPEG encoder consumes
/// directly. `rgb_scratch` and `jpeg_scratch` are caller-owned
/// reusable buffers so the per-frame path stays alloc-free after
/// the first frame sizes them.
fn encode_and_write(
    frame: &scap::frame::Frame,
    writer: &mut BufWriter<File>,
    rgb_scratch: &mut Vec<u8>,
    jpeg_scratch: &mut Vec<u8>,
) -> Result<(), String> {
    // `scap::frame::Frame` is an enum across pixel layouts; we
    // requested `BGRAFrame` in `Options.output_type` so any other
    // variant is a programming error.
    let bgra = match frame {
        scap::frame::Frame::BGRA(f) => f,
        other => {
            return Err(format!(
                "unexpected frame variant: {:?}",
                std::mem::discriminant(other)
            ));
        },
    };

    let width = bgra.width as u32;
    let height = bgra.height as u32;
    let rgb_len = (width as usize) * (height as usize) * 3;

    // BGRA → RGB swizzle into the reusable scratch buffer. We can't
    // share with `image` (RgbImage owns its data), so the swizzle is
    // unavoidable — but the allocation is amortised across the recording.
    rgb_scratch.clear();
    rgb_scratch.reserve(rgb_len);
    for chunk in bgra.data.as_chunks::<4>().0 {
        // chunk = [B, G, R, A]; we want [R, G, B].
        rgb_scratch.push(chunk[2]);
        rgb_scratch.push(chunk[1]);
        rgb_scratch.push(chunk[0]);
    }

    // `RgbImage::from_raw` requires owned `Vec<u8>` — std::mem::take
    // moves the scratch out (replacing it with an empty Vec, which we
    // restore below) so we don't pay for an extra clone.
    let owned_rgb = std::mem::take(rgb_scratch);
    let img = image::RgbImage::from_raw(width, height, owned_rgb)
        .ok_or_else(|| "RgbImage::from_raw size mismatch".to_string())?;
    let dyn_img = image::DynamicImage::ImageRgb8(img);

    // Encode with quality 80 — the conventional MJPEG default,
    // good visual quality at ~10:1 compression.
    jpeg_scratch.clear();
    dyn_img
        .write_to(
            &mut std::io::Cursor::new(&mut *jpeg_scratch),
            image::ImageFormat::Jpeg,
        )
        .map_err(|e| format!("jpeg encode: {e}"))?;

    writer
        .write_all(jpeg_scratch)
        .map_err(|e| format!("write frame: {e}"))?;

    // Reclaim the moved-out buffer so the next frame doesn't allocate.
    // `into_raw` returns the underlying Vec (after `image` consumed it
    // through DynamicImage); take it back for re-use.
    *rgb_scratch = dyn_img.into_rgb8().into_raw();
    Ok(())
}
