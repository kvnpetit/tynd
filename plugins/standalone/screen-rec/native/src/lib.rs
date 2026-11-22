//! Cross-platform screen recording via [`scap`] (ScreenCaptureKit /
//! DXGI / PipeWire / XCB).
//!
//! `--features capture` pulls in the `scap` backend; without it every
//! method returns a "rebuild with --features capture" error.
//!
//! Two encoders ship in-tree: `mjpeg_backend` (pure Rust via `image`,
//! zero system deps, default) and `h264_backend` (`--features
//! gpu-encode`, via `ffmpeg-next`, drives NVENC / VideoToolbox / QSV /
//! VA-API with a libx264 CPU fallback).
//!
//! Permissions: macOS goes through `scap::has_permission()` /
//! `request_permission()`; Windows may show a one-shot UAC dialog;
//! PipeWire handles auth on Wayland.

#![forbid(unsafe_code)]
#![deny(unsafe_op_in_unsafe_fn)]

use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::json;
use specta::Type;
use tynd_plugin_sdk::{
    tynd_plugin::{plugin, plugin_impl},
    PluginCx, PluginError, PluginResult, Registry,
};

const MAX_RECORDINGS: usize = 8;

fn default_fps() -> u32 {
    30
}

fn default_bitrate() -> u32 {
    8_000
}

/// Capture region in physical screen pixels. Origin top-left.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Region {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

/// `Mjpeg` always available; `H264` / `Hevc` need `gpu-encode`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum Codec {
    #[default]
    Mjpeg,
    H264,
    Hevc,
}

/// `Auto` walks NVENC → VideoToolbox → QSV → VA-API → libx264.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum EncoderChoice {
    #[default]
    Auto,
    Nvenc,
    Videotoolbox,
    Qsv,
    Vaapi,
    X264,
    X265,
}

/// Maps onto the ffmpeg `-preset` family (normalised per encoder in
/// `h264_backend`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum Preset {
    Ultrafast,
    #[default]
    Fast,
    Medium,
    Slow,
}

/// `output` is the only required field; bitrate/preset ignored for MJPEG.
#[derive(Debug, Clone, Serialize, Deserialize, Type, bon::Builder)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct RecordingOpts {
    #[serde(default = "default_fps")]
    #[builder(default = default_fps())]
    pub fps: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub region: Option<Region>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub monitor: Option<u32>,
    #[serde(default)]
    #[builder(default)]
    pub audio: bool,
    /// Output container; defaults to the codec's natural extension.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    #[builder(into)]
    pub output: String,
    #[serde(default)]
    #[builder(default)]
    pub codec: Codec,
    #[serde(default)]
    #[builder(default)]
    pub encoder: EncoderChoice,
    /// kbps; default 8000.
    #[serde(default = "default_bitrate")]
    #[builder(default = default_bitrate())]
    pub bitrate: u32,
    #[serde(default)]
    #[builder(default)]
    pub preset: Preset,
}

/// `id` rendered as JS `number`; `startedAt` is unix epoch ms.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RecordingHandle {
    #[specta(type = f64)]
    pub id: u64,
    #[specta(type = f64)]
    pub started_at: i64,
    pub output: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RecordingReport {
    #[specta(type = f64)]
    pub duration_ms: u64,
    #[specta(type = f64)]
    pub size_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MonitorInfo {
    pub id: u32,
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub primary: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct WindowInfo {
    pub id: u32,
    pub title: String,
    pub app_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Sources {
    pub monitors: Vec<MonitorInfo>,
    pub windows: Vec<WindowInfo>,
}

mod backend;

fn validate_recording_opts(opts: &RecordingOpts) -> PluginResult<()> {
    if !(1..=240).contains(&opts.fps) {
        return Err(PluginError::InvalidArg {
            key: "opts.fps".into(),
            detail: "screen-rec.start: fps must be between 1 and 240".into(),
        });
    }
    if let Some(region) = opts.region {
        if region.width == 0
            || region.height == 0
            || region.width > 16_384
            || region.height > 16_384
        {
            return Err(PluginError::InvalidArg {
                key: "opts.region".into(),
                detail: "screen-rec.start: region dimensions must be between 1 and 16384".into(),
            });
        }
    }
    if opts.audio {
        return Err(PluginError::InvalidArg {
            key: "opts.audio".into(),
            detail: "screen-rec.start: audio capture is not implemented".into(),
        });
    }
    if opts.format.is_some() {
        return Err(PluginError::InvalidArg {
            key: "opts.format".into(),
            detail: "screen-rec.start: format override is not implemented; codec selects output"
                .into(),
        });
    }
    Ok(())
}

/// Active (non-paused) recording time: wall time since start minus the
/// current pause (if any) minus all earlier pauses.
fn active_duration(
    elapsed: Duration,
    current_pause: Option<Duration>,
    paused_for: Duration,
) -> Duration {
    elapsed
        .saturating_sub(current_pause.unwrap_or(Duration::ZERO))
        .saturating_sub(paused_for)
}

/// Snapshot the active duration, stop and join the encoder, then report
/// the flushed file size. Worker failures propagate to the caller.
fn stop_recorder(rec: &mut backend::Recorder) -> PluginResult<RecordingReport> {
    let elapsed = active_duration(
        rec.started_at.elapsed(),
        rec.paused_at
            .take()
            .map(|pause_start| pause_start.elapsed()),
        rec.paused_for,
    );
    backend::stop(rec)?;
    let duration_ms = u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX);
    let size_bytes = std::fs::metadata(&rec.output)
        .map_err(|error| {
            PluginError::Custom(format!(
                "screen-rec.stop: stat output '{}': {error}",
                rec.output
            ))
        })?
        .len();
    Ok(RecordingReport {
        duration_ms,
        size_bytes,
    })
}

/// Stop and finalize every live recording (flush the MJPEG writer / write the
/// H.264 trailer) and drop it from the registry. The host never drops plugin
/// state at exit, so this is the only chance to produce playable files.
///
/// Recordings are finalized in parallel so one slow encoder cannot multiply
/// the shutdown latency. Returns one message per failed recording.
fn shutdown_recordings(recordings: &Registry<backend::Recorder>) -> Vec<String> {
    let ids = recordings.ids();
    let mut failures = Vec::new();
    std::thread::scope(|scope| {
        let workers: Vec<_> = ids
            .into_iter()
            .map(|id| {
                let worker = scope.spawn(move || {
                    let result = recordings.with(id, |rec| {
                        let result = stop_recorder(rec);
                        if result.is_err() {
                            // Wedged encoder: one 5 s wait is enough at exit;
                            // `Recorder::drop` must not wait again.
                            backend::detach(rec);
                        }
                        result
                    });
                    recordings.remove(id);
                    result
                });
                (id, worker)
            })
            .collect();
        for (id, worker) in workers {
            match worker.join() {
                Ok(Ok(_)) => {},
                Ok(Err(error)) => failures.push(format!("recording {id}: {error}")),
                Err(_) => failures.push(format!("recording {id}: finalizer panicked")),
            }
        }
    });
    failures
}

#[plugin(name = "screen-rec")]
pub struct ScreenRecPlugin {
    recordings: Registry<backend::Recorder>,
}

impl std::fmt::Debug for ScreenRecPlugin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScreenRecPlugin").finish_non_exhaustive()
    }
}

impl Default for ScreenRecPlugin {
    fn default() -> Self {
        Self {
            recordings: Registry::new(MAX_RECORDINGS),
        }
    }
}

fn check_screen_capture(cx: &PluginCx) -> PluginResult<()> {
    // Screen capture is sensitive; absence of a policy dispatcher is denial.
    cx.os_call("security", "checkScreenCapture", json!({}))?;
    Ok(())
}

// `#[command]` requires `&self`; discovery and permissions are backend-owned.
#[allow(clippy::unused_self)]
#[plugin_impl]
impl ScreenRecPlugin {
    #[command]
    pub fn list_sources(&self, cx: &PluginCx) -> PluginResult<Sources> {
        check_screen_capture(cx)?;
        backend::list_sources()
    }

    #[command]
    pub fn has_permission(&self, cx: &PluginCx) -> PluginResult<bool> {
        check_screen_capture(cx)?;
        Ok(backend::has_permission())
    }

    #[command]
    pub fn request_permission(&self, cx: &PluginCx) -> PluginResult<bool> {
        check_screen_capture(cx)?;
        Ok(backend::request_permission())
    }

    #[command]
    pub fn start(&self, cx: &PluginCx, mut opts: RecordingOpts) -> PluginResult<RecordingHandle> {
        check_screen_capture(cx)?;
        validate_recording_opts(&opts)?;
        // Reject unsupported codec *before* spinning up the capturer.
        if !backend::codec_supported(opts.codec) {
            return Err(PluginError::Custom(format!(
                "screen-rec: codec {:?} requires the `gpu-encode` Cargo feature. Rebuild with:\n  \
                 cd plugins/standalone/screen-rec/native && cargo build --features gpu-encode\nOr use codec: \
                 'mjpeg' (always available).",
                opts.codec
            )));
        }
        opts.output = cx.authorize_fs_path(&opts.output, "write")?;
        let output = opts.output.clone();
        let recorder = backend::start(opts)?;
        // Read back the wall-clock snapshot the recorder took at
        // construction so `list()` and `start()` agree on the exact
        // value (no second `SystemTime::now()` call drifting from the
        // first by a few µs).
        let started_at = recorder.started_at_ms;
        let id = self.recordings.insert(recorder)?;
        Ok(RecordingHandle {
            id,
            started_at,
            output,
        })
    }

    #[command]
    pub fn stop(&self, recording_id: u64) -> PluginResult<RecordingReport> {
        let stop_result = self.recordings.with_arg(
            recording_id,
            "recordingId",
            "screen-rec.stop",
            stop_recorder,
        );
        self.recordings.remove(recording_id);
        stop_result
    }

    #[command]
    pub fn pause(&self, recording_id: u64) -> PluginResult<()> {
        self.recordings
            .with_arg(recording_id, "recordingId", "screen-rec.pause", |rec| {
                if rec.paused_at.is_none() {
                    rec.paused_at = Some(Instant::now());
                }
                #[cfg(feature = "capture")]
                rec.set_paused(true);
                Ok(())
            })
    }

    #[command]
    pub fn resume(&self, recording_id: u64) -> PluginResult<()> {
        self.recordings
            .with_arg(recording_id, "recordingId", "screen-rec.resume", |rec| {
                if let Some(pause_start) = rec.paused_at.take() {
                    rec.paused_for = rec.paused_for.saturating_add(pause_start.elapsed());
                }
                #[cfg(feature = "capture")]
                rec.set_paused(false);
                Ok(())
            })
    }

    #[command]
    pub fn list(&self) -> PluginResult<Vec<RecordingHandle>> {
        // Entries removed mid-iteration are skipped by `snapshot`.
        let mut out = Vec::new();
        let mut failed = Vec::new();
        for entry in self.recordings.snapshot(|id, rec| {
            Ok(match backend::failure(rec) {
                Some(error) => Err((id, error)),
                None => Ok(RecordingHandle {
                    id,
                    started_at: rec.started_at_ms,
                    output: rec.output.clone(),
                }),
            })
        }) {
            match entry {
                Ok(handle) => out.push(handle),
                Err(failure) => failed.push(failure),
            }
        }
        let failure_summary = failed
            .iter()
            .map(|(id, error)| format!("recording {id}: {error}"))
            .collect::<Vec<_>>()
            .join("; ");
        for (id, error) in failed {
            tracing::error!(target: "tynd::plugin::screen_rec", id, "recording failed: {error}");
            self.recordings.remove(id);
        }
        if failure_summary.is_empty() {
            Ok(out)
        } else {
            Err(PluginError::Custom(format!(
                "screen-rec.list: {failure_summary}"
            )))
        }
    }

    /// Native host lifecycle hook. Finalizes every active recording so the
    /// output files are flushed and playable after a graceful exit.
    pub fn plugin_shutdown(&self, _cx: &PluginCx) -> PluginResult<()> {
        let failures = shutdown_recordings(&self.recordings);
        if failures.is_empty() {
            Ok(())
        } else {
            Err(PluginError::Custom(format!(
                "screen-rec.shutdown: {}",
                failures.join("; ")
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts() -> RecordingOpts {
        RecordingOpts::builder().output("out.mjpeg").build()
    }

    #[test]
    fn validate_accepts_defaults() {
        assert!(validate_recording_opts(&opts()).is_ok());
    }

    #[test]
    fn validate_rejects_bad_fps() {
        for fps in [0, 241] {
            let mut o = opts();
            o.fps = fps;
            assert!(matches!(
                validate_recording_opts(&o),
                Err(PluginError::InvalidArg { .. })
            ));
        }
    }

    #[test]
    fn validate_rejects_bad_region_audio_format() {
        let mut o = opts();
        o.region = Some(Region {
            x: 0,
            y: 0,
            width: 0,
            height: 10,
        });
        assert!(validate_recording_opts(&o).is_err());
        o.region = Some(Region {
            x: 0,
            y: 0,
            width: 16_385,
            height: 10,
        });
        assert!(validate_recording_opts(&o).is_err());
        o.region = Some(Region {
            x: -5,
            y: -5,
            width: 100,
            height: 100,
        });
        assert!(validate_recording_opts(&o).is_ok());

        let mut o = opts();
        o.audio = true;
        assert!(validate_recording_opts(&o).is_err());
        let mut o = opts();
        o.format = Some("mp4".into());
        assert!(validate_recording_opts(&o).is_err());
    }

    #[test]
    fn active_duration_subtracts_pauses() {
        let s = Duration::from_secs;
        assert_eq!(active_duration(s(10), None, Duration::ZERO), s(10));
        assert_eq!(active_duration(s(10), Some(s(3)), Duration::ZERO), s(7));
        assert_eq!(active_duration(s(10), Some(s(3)), s(2)), s(5));
        assert_eq!(active_duration(s(10), None, s(4)), s(6));
    }

    #[test]
    fn active_duration_saturates() {
        let s = Duration::from_secs;
        assert_eq!(active_duration(s(1), Some(s(5)), s(5)), Duration::ZERO);
    }

    #[cfg(not(feature = "capture"))]
    #[test]
    fn shutdown_finalizes_and_clears_every_recording() {
        let dir = std::env::temp_dir().join(format!("tynd-screen-rec-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let registry: Registry<backend::Recorder> = Registry::new(MAX_RECORDINGS);
        let mut paths = Vec::new();
        for i in 0..3 {
            let path = dir.join(format!("rec{i}.bin"));
            std::fs::write(&path, b"data").unwrap();
            let output = path.to_string_lossy().into_owned();
            registry
                .insert(backend::Recorder {
                    started_at: Instant::now(),
                    started_at_ms: 0,
                    output,
                    paused_for: Duration::ZERO,
                    paused_at: Some(Instant::now()),
                })
                .unwrap();
            paths.push(path);
        }
        let failures = shutdown_recordings(&registry);
        assert!(failures.is_empty(), "{failures:?}");
        assert!(registry.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(not(feature = "capture"))]
    #[test]
    fn shutdown_reports_missing_output_and_still_clears() {
        let registry: Registry<backend::Recorder> = Registry::new(MAX_RECORDINGS);
        registry
            .insert(backend::Recorder {
                started_at: Instant::now(),
                started_at_ms: 0,
                output: "definitely/does/not/exist.bin".into(),
                paused_for: Duration::ZERO,
                paused_at: None,
            })
            .unwrap();
        let failures = shutdown_recordings(&registry);
        assert_eq!(failures.len(), 1);
        assert!(registry.is_empty());
    }
}
