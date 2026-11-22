//! H.264 / HEVC encoder backend — wraps `ffmpeg-next` to drive the
//! OS-native hardware encoders shipped with every modern GPU.
//!
//! ## Encoder selection
//!
//! `RecordingOpts::encoder` picks the backend explicitly; `Auto`
//! walks a priority list of GPU-accelerated encoders and falls back
//! to `libx264` (CPU) if none of them are available in the linked
//! ffmpeg build:
//!
//! 1. `h264_nvenc`        — NVIDIA GPUs (Windows / Linux)
//! 2. `h264_videotoolbox` — Apple silicon + Intel Macs
//! 3. `h264_qsv`          — Intel CPUs with a recent iGPU
//! 4. `h264_vaapi`        — AMD / Intel on Linux (DRM stack)
//! 5. `libx264`           — CPU fallback, ships with every libavcodec
//!
//! HEVC mirrors the same list (`hevc_nvenc`, `hevc_videotoolbox`, …,
//! `libx265`).
//!
//! ## Pipeline
//!
//! Per frame:
//!
//! 1. The capture thread pulls BGRA frames from `scap`; the worker polls them with a timeout.
//! 2. The frame is wrapped in an `ffmpeg::frame::Video` with format `BGRA` and the source
//!    dimensions — no copy of the pixel data, just a view backed by `frame.data_mut(0)`.
//! 3. A cached `software::scaling::Context` converts BGRA → YUV420P. NVENC / VideoToolbox
//!    technically accept NV12 directly, but going through YUV420P keeps the code path uniform
//!    across encoders and the swscale step is cheap on a worker thread.
//! 4. `encoder.send_frame(&yuv)` then a `receive_packet()` loop drains every packet ffmpeg has
//!    ready, rescales the timestamps into the muxer's time base, and writes them out.
//!
//! On stop, we send EOF, drain the encoder one last time, and call
//! `octx.write_trailer()` so the resulting MP4 / MKV / MOV is a valid
//! seekable container (without the trailer the file is truncated and
//! most players refuse to open it).
//!
//! ## Lazy resolution detection
//!
//! `scap` reports the actual capture resolution only once the first
//! frame lands — the requested `output_resolution` is best-effort and
//! the platform may scale to a different size (DPI scaling on Windows,
//! Retina downscaling on macOS). To avoid silently dropping every
//! frame because of a 1080p hardcode that doesn't match reality, the
//! ffmpeg encoder + scaler are built lazily from the first frame's
//! dimensions inside the worker thread. The synchronous `spawn` path
//! still validates the encoder name, output path + opens the muxer
//! container so the obvious "no such codec / can't write here" errors
//! surface to the JS caller before any thread starts pumping.

use std::{
    path::PathBuf,
    sync::atomic::{AtomicBool, Ordering},
};

use ffmpeg_next::{self as ffmpeg};
use tynd_plugin_sdk::{PluginError, PluginResult};

use super::{
    merge_worker_results, spawn_encoder_thread, Codec, EncoderChoice, EncoderThread, FrameSource,
    NextFrame, Preset, RecordingOpts, FRAME_POLL,
};

#[path = "h264_backend/pipeline.rs"]
mod pipeline;

use pipeline::Pipeline;

/// Spawn the H.264 / HEVC encoder thread. Resolves the encoder name
/// and validates the output path up-front (so config errors surface
/// synchronously to the JS caller instead of being swallowed by the
/// worker thread). The worker constructs the capturer itself because
/// Windows capture handles are thread-affine and `Capturer` is not
/// `Send` there. The rest of the pipeline is built lazily from the
/// first frame's real dimensions — see the module docs for why.
pub(super) fn spawn(opts: &RecordingOpts) -> PluginResult<EncoderThread> {
    // Initialise ffmpeg once — `init()` is idempotent and cheap on
    // subsequent calls, so we don't need a `OnceLock`.
    if let Err(e) = ffmpeg::init() {
        return Err(PluginError::Custom(format!("screen-rec: ffmpeg init: {e}")));
    }

    let codec_name = match pick_encoder(opts.codec, opts.encoder) {
        Some(name) => name,
        None => {
            return Err(PluginError::Custom(format!(
                "screen-rec: no {:?} encoder available in this ffmpeg build — rebuild ffmpeg with \
                 --enable-libx264 / --enable-nvenc / --enable-videotoolbox or pick a different \
                 `encoder` in RecordingOpts",
                opts.codec
            )));
        },
    };

    // Cheap synchronous probe of the output path so a missing parent
    // directory or read-only filesystem fails fast. We don't actually
    // open the muxer here — the muxer needs the real width/height to
    // write a usable header, and those only land with the first frame.
    let output_path = PathBuf::from(&opts.output);
    if let Some(parent) = output_path.parent() {
        if !parent.as_os_str().is_empty() && !parent.exists() {
            return Err(PluginError::Custom(format!(
                "screen-rec: output directory {} does not exist",
                parent.display()
            )));
        }
    }

    let cfg = PipelineConfig {
        output: output_path,
        codec_name,
        fps: opts.fps.max(1),
        bitrate_kbps: opts.bitrate,
        preset: opts.preset,
    };

    spawn_encoder_thread(
        "tynd-screen-rec-h264",
        "h264",
        opts,
        move |source, running, paused| run_loop(&source, cfg, &running, &paused),
    )
}

/// Return the ffmpeg encoder name for the requested codec + backend
/// combo, or `None` if nothing on the priority list is available in
/// this build of libavcodec.
fn pick_encoder(codec: Codec, choice: EncoderChoice) -> Option<&'static str> {
    let candidates: &[&'static str] = match (codec, choice) {
        // Explicit picks — single-shot, no fallback chain so the
        // caller knows immediately if their preferred encoder is
        // missing.
        (Codec::H264, EncoderChoice::Nvenc) => &["h264_nvenc"],
        (Codec::H264, EncoderChoice::Videotoolbox) => &["h264_videotoolbox"],
        (Codec::H264, EncoderChoice::Qsv) => &["h264_qsv"],
        (Codec::H264, EncoderChoice::Vaapi) => &["h264_vaapi"],
        (Codec::H264, EncoderChoice::X264) => &["libx264"],
        (Codec::H264, EncoderChoice::X265) => &["libx264"], // mismatch — coerce to libx264

        (Codec::Hevc, EncoderChoice::Nvenc) => &["hevc_nvenc"],
        (Codec::Hevc, EncoderChoice::Videotoolbox) => &["hevc_videotoolbox"],
        (Codec::Hevc, EncoderChoice::Qsv) => &["hevc_qsv"],
        (Codec::Hevc, EncoderChoice::Vaapi) => &["hevc_vaapi"],
        (Codec::Hevc, EncoderChoice::X264) => &["libx265"], // mismatch — coerce to libx265
        (Codec::Hevc, EncoderChoice::X265) => &["libx265"],

        // Auto — walk the GPU encoders first, fall back to CPU.
        (Codec::H264, EncoderChoice::Auto) => &[
            "h264_nvenc",
            "h264_videotoolbox",
            "h264_qsv",
            "h264_vaapi",
            "libx264",
        ],
        (Codec::Hevc, EncoderChoice::Auto) => &[
            "hevc_nvenc",
            "hevc_videotoolbox",
            "hevc_qsv",
            "hevc_vaapi",
            "libx265",
        ],

        // MJPEG goes to the other backend — this branch is unreachable
        // because the caller filters on codec before getting here, but
        // returning None keeps the match exhaustive without a panic.
        (Codec::Mjpeg, _) => &[],
    };

    candidates
        .iter()
        .copied()
        .find(|name| ffmpeg::encoder::find_by_name(name).is_some())
}

/// Synchronously-resolved settings the worker thread needs to build
/// the ffmpeg pipeline once it sees the first frame's real width and
/// height. Anything that depends on the captured resolution is built
/// lazily inside `Pipeline::new`; everything else lives here so its
/// validation can happen before we spin up the worker.
struct PipelineConfig {
    output: PathBuf,
    codec_name: &'static str,
    fps: u32,
    bitrate_kbps: u32,
    preset: Preset,
}

/// Worker thread — pumps frames from `scap` into the H.264 pipeline
/// until `running` flips. The first frame's dimensions are used to
/// build the ffmpeg pipeline lazily (see module docs); every
/// subsequent frame goes through the same pipeline. On exit the
/// pipeline is explicitly finished so encoder and trailer failures reach
/// the plugin caller. `Drop` remains a last-resort cleanup path for panics.
fn run_loop(
    source: &FrameSource,
    cfg: PipelineConfig,
    running: &AtomicBool,
    paused: &AtomicBool,
) -> Result<(), String> {
    let mut pipeline: Option<Pipeline> = None;
    let work_result = (|| {
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
            let bgra = match &frame {
                scap::frame::Frame::BGRA(frame) => frame,
                other => {
                    return Err(format!(
                        "unexpected frame variant: {:?}",
                        std::mem::discriminant(other)
                    ));
                },
            };

            let width = bgra.width as u32;
            let height = bgra.height as u32;
            if pipeline.is_none() {
                pipeline = Some(
                    Pipeline::new(
                        &cfg.output,
                        cfg.codec_name,
                        width,
                        height,
                        cfg.fps,
                        cfg.bitrate_kbps,
                        cfg.preset,
                    )
                    .map_err(|error| format!("lazy pipeline init at {width}x{height}: {error}"))?,
                );
            }
            pipeline
                .as_mut()
                .ok_or_else(|| "screen-rec: pipeline did not initialize".to_string())?
                .push_bgra(&bgra.data, width, height)
                .map_err(|error| format!("encode frame: {error}"))?;
        }
        Ok(())
    })();
    let finish_result = pipeline.map_or(Ok(()), |mut pipeline| pipeline.finish());
    merge_worker_results(work_result, finish_result)
}
