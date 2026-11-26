//! Audio capture & playback via `cpal` + WAV I/O via `hound`.
//!
//! Feature gates: `--features capture` pulls in `cpal` + `hound`;
//! `--features wav` pulls in `hound` alone for one-shot `saveWav`.
//! Without them every dispatch arm returns a "rebuild with --features
//! <name>" error.
//!
//! Capture streams emit `audio:samples` events with base64-encoded f32
//! samples. Playback returns an id for `stopPlayback`. WAV save is
//! one-shot.
//!
//! `cpal::Stream` is `!Send`, so all live streams live on a dedicated
//! audio control thread that owns them and processes `Start` / `Stop`
//! over a crossbeam channel. Dispatch arms are pure producers.

#![forbid(unsafe_code)]

// `audio:samples` payloads cast `&[f32]` to `&[u8]` and base64-encode
// the result. JS-side `DataView` decode assumes little-endian, so the
// encoder pipeline only round-trips on LE targets. Fail the build on a
// big-endian target rather than silently shipping a broken stream.
#[cfg(target_endian = "big")]
compile_error!("audio plugin requires little-endian target");

use std::sync::atomic::{AtomicU64, Ordering};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::json;
use specta::Type;
use tynd_plugin_sdk::{
    tynd_plugin::{plugin, plugin_impl},
    BoundedPermitPool, Permit, PluginCx, PluginError, PluginResult, DEFAULT_PERMIT_WAIT,
};

/// A live-playback slot: held by the playback until it stops or ends.
type PlaybackSlot = Permit;

/// All fields optional; missing values inherit platform defaults.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type, bon::Builder)]
#[serde(rename_all = "camelCase")]
pub struct CaptureOpts {
    #[specta(type = Option<f64>)]
    pub sample_rate: Option<u32>,
    #[specta(type = Option<f64>)]
    pub channels: Option<u16>,
    #[builder(into)]
    pub format: Option<String>,
    #[builder(into)]
    pub device: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PlayFileOpts {
    pub volume: Option<f32>,
    pub device_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PlaySamplesOpts {
    #[specta(type = f64)]
    pub sample_rate: u32,
    #[specta(type = f64)]
    pub channels: u16,
    pub volume: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SaveWavOpts {
    #[specta(type = f64)]
    pub sample_rate: u32,
    #[specta(type = f64)]
    pub channels: u16,
}

/// Listed input/output device names exposed by the OS audio host.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ListDevicesResult {
    pub inputs: Vec<String>,
    pub outputs: Vec<String>,
}

/// Capture handle id; userland feeds it back into `stopCapture`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct StartCaptureResult {
    #[specta(type = f64)]
    pub id: u64,
}

/// Playback handle + duration estimate for `playFile`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PlayFileResult {
    #[specta(type = f64)]
    pub id: u64,
    #[specta(type = f64)]
    pub duration_ms: u64,
}

/// Playback handle for `playSamples`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PlaySamplesResult {
    #[specta(type = f64)]
    pub id: u64,
}

// Event payload shapes (`audio:samples`, `audio:warn`, `audio:error`)
// are documented + typed in the frontend at `plugins/standalone/audio/client.ts` —
// specta only walks types reachable from `#[command]` signatures, so
// these are emitted via `serde_json::json!` rather than being defined
// as Rust structs here.

/// Rejects sample buffers that cannot describe whole interleaved frames.
fn validate_audio_format(
    operation: &str,
    sample_rate: u32,
    channels: u16,
    sample_count: usize,
) -> PluginResult<()> {
    if sample_rate == 0 {
        return Err(PluginError::InvalidArg {
            key: "opts.sampleRate".into(),
            detail: format!("{operation}: sampleRate must be greater than zero"),
        });
    }
    if channels == 0 {
        return Err(PluginError::InvalidArg {
            key: "opts.channels".into(),
            detail: format!("{operation}: channels must be greater than zero"),
        });
    }
    if !sample_count.is_multiple_of(usize::from(channels)) {
        return Err(PluginError::InvalidArg {
            key: "samples".into(),
            detail: format!(
                "{operation}: sample count {sample_count} is not a multiple of {channels} channels"
            ),
        });
    }
    Ok(())
}

/// Owns the cross-feature id allocator so playback / capture ids stay
/// unique. The cpal stream registry lives in `capture_backend` (process-
/// globally lazy-init'd).
#[plugin(name = "audio")]
pub struct AudioPlugin {
    next_id: AtomicU64,
    _state: Mutex<()>,
    /// Bounded permit pool capping the *decode* phase of `playFile` /
    /// `playSamples` at `available_parallelism()`. Each decode allocates
    /// a `Vec<f32>` up to the 256 MiB per-call cap; without this gate N
    /// concurrent decodes could allocate N × 256 MiB. Released once the
    /// stream is built — see `playback_slots` for a separate bound on
    /// concurrent live streams.
    decode_permits: BoundedPermitPool,

    /// Bounded slot pool capping concurrent *live* playbacks. The decode
    /// permit only covers the decode + stream-build window (~ms); a long
    /// WAV holds memory for its entire duration. Slot guards are dropped
    /// on `stopPlayback`. Sized at 4× decode permits — playback is mostly
    /// idle DMA, so the host can hold more streams than active decodes.
    playback_slots: BoundedPermitPool,
}

impl std::fmt::Debug for AudioPlugin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AudioPlugin").finish_non_exhaustive()
    }
}

impl Default for AudioPlugin {
    fn default() -> Self {
        let decode_permits = BoundedPermitPool::per_core("audio decode permit pool");
        // 4× decode permits — see field doc.
        let playback_slots = BoundedPermitPool::new(
            decode_permits.capacity().saturating_mul(4),
            "audio playback slots",
        );
        Self {
            next_id: AtomicU64::new(1),
            _state: Mutex::new(()),
            decode_permits,
            playback_slots,
        }
    }
}

impl AudioPlugin {
    fn alloc_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    fn acquire_playback_permits(&self, operation: &str) -> PluginResult<(PlaybackSlot, Permit)> {
        // `try_acquire`: a full pool fails fast instead of parking a shared
        // call-pool worker until some other playback ends.
        let slot = self.playback_slots.try_acquire(operation)?.ok_or_else(|| {
            PluginError::Custom(format!(
                "{operation}: too many live playbacks ({}); stop one first",
                self.playback_slots.capacity()
            ))
        })?;
        let decode = self
            .decode_permits
            .acquire_timeout(operation, DEFAULT_PERMIT_WAIT)?;
        Ok((slot, decode))
    }
}

// `#[command]` requires `&self`; several commands intentionally use only shared backend state.
#[allow(clippy::unused_self)]
#[plugin_impl]
impl AudioPlugin {
    /// List every input + output device the OS audio host advertises.
    #[command]
    pub fn list_devices(&self) -> PluginResult<ListDevicesResult> {
        let d = backend::list_devices()?;
        Ok(ListDevicesResult {
            inputs: d.inputs,
            outputs: d.outputs,
        })
    }

    /// Open an input stream and start emitting `audio:samples` events.
    #[command]
    pub fn start_capture(
        &self,
        cx: &PluginCx,
        opts: Option<CaptureOpts>,
    ) -> PluginResult<StartCaptureResult> {
        let opts = opts.unwrap_or_default();
        // Capture output is always little-endian f32; refuse formats we would
        // silently ignore.
        if let Some(format) = opts.format.as_deref() {
            if format != "f32" {
                return Err(PluginError::InvalidArg {
                    key: "opts.format".into(),
                    detail: format!(
                        "audio.startCapture: unsupported format '{format}'; only 'f32' is supported"
                    ),
                });
            }
        }
        // Audio capture is sensitive: absence of the security dispatcher
        // must deny, never silently grant.
        cx.os_call(
            "security",
            "checkAudioInput",
            json!({ "device": opts.device }),
        )?;
        let id = backend::start_capture(self.alloc_id(), opts, cx.emitter())?;
        Ok(StartCaptureResult { id })
    }

    /// Idempotent — stopping an unknown id is a no-op.
    #[command]
    pub fn stop_capture(&self, id: u64) -> PluginResult<()> {
        backend::stop_capture(id)
    }

    /// Decode `path` (any cpal-supported WAV variant) and play it back.
    /// Returns the playback id + an estimate of total duration.
    #[command]
    pub fn play_file(
        &self,
        cx: &PluginCx,
        path: String,
        opts: Option<PlayFileOpts>,
    ) -> PluginResult<PlayFileResult> {
        let opts = opts.unwrap_or_default();
        cx.os_call(
            "security",
            "checkAudioOutput",
            json!({ "device": &opts.device_id }),
        )?;
        let authorized_path = cx.authorize_fs_path(&path, "read")?;
        // Live slot survives until stop/EOF; decode permit ends with this dispatch.
        let (slot, _decode) = self.acquire_playback_permits("audio.playFile")?;
        let (id, duration_ms) =
            backend::play_file(self.alloc_id(), &authorized_path, opts, cx.emitter(), slot)?;
        Ok(PlayFileResult { id, duration_ms })
    }

    /// Play raw interleaved f32 samples through the default output device.
    #[command]
    pub fn play_samples(
        &self,
        cx: &PluginCx,
        samples: Vec<f32>,
        opts: PlaySamplesOpts,
    ) -> PluginResult<PlaySamplesResult> {
        // `playSamples` doesn't take a device id today — gate on
        // unspecified output. Missing security dispatch is a hard deny.
        cx.os_call(
            "security",
            "checkAudioOutput",
            json!({ "device": serde_json::Value::Null }),
        )?;
        validate_audio_format(
            "audio.playSamples",
            opts.sample_rate,
            opts.channels,
            samples.len(),
        )?;
        // Mirror of `play_file`: live slot + dispatch-scoped decode permit.
        let (slot, _decode) = self.acquire_playback_permits("audio.playSamples")?;
        let id = backend::play_samples(self.alloc_id(), samples, opts, cx.emitter(), slot)?;
        Ok(PlaySamplesResult { id })
    }

    /// Idempotent — stopping an unknown id is a no-op.
    #[command]
    pub fn stop_playback(&self, id: u64) -> PluginResult<()> {
        backend::stop_playback(id)
    }

    /// Write `samples` (interleaved f32) to `path` as a 32-bit float WAV.
    #[command]
    pub fn save_wav(
        &self,
        cx: &PluginCx,
        samples: Vec<f32>,
        path: String,
        opts: SaveWavOpts,
    ) -> PluginResult<()> {
        validate_audio_format(
            "audio.saveWav",
            opts.sample_rate,
            opts.channels,
            samples.len(),
        )?;
        let authorized_path = cx.authorize_fs_path(&path, "write")?;
        backend::save_wav(&samples, &authorized_path, opts)
    }

    /// Native host lifecycle hook. Stops every live stream and joins the
    /// control thread before process teardown.
    pub fn plugin_shutdown(&self, _cx: &PluginCx) -> PluginResult<()> {
        backend::shutdown()
    }
}

#[cfg(feature = "capture")]
#[path = "capture_backend/enabled.rs"]
mod capture_backend;

#[cfg(not(feature = "capture"))]
#[path = "capture_backend/disabled.rs"]
mod capture_backend;
#[cfg(feature = "wav")]
mod wav_backend {
    use std::path::Path;

    use hound::{SampleFormat, WavSpec, WavWriter};

    use super::{PluginError, PluginResult, SaveWavOpts};

    pub(crate) fn save_wav(samples: &[f32], path: &str, opts: SaveWavOpts) -> PluginResult<()> {
        let spec = WavSpec {
            channels: opts.channels,
            sample_rate: opts.sample_rate,
            bits_per_sample: 32,
            sample_format: SampleFormat::Float,
        };
        let mut writer = WavWriter::create(Path::new(path), spec)
            .map_err(|e| PluginError::Custom(format!("audio.saveWav(create '{path}'): {e}")))?;
        for &s in samples {
            writer
                .write_sample(s)
                .map_err(|e| PluginError::Custom(format!("audio.saveWav(write): {e}")))?;
        }
        writer
            .finalize()
            .map_err(|e| PluginError::Custom(format!("audio.saveWav(finalize): {e}")))?;
        Ok(())
    }
}

#[cfg(not(feature = "wav"))]
mod wav_backend {
    use super::{PluginError, PluginResult, SaveWavOpts};

    pub(crate) fn save_wav(_samples: &[f32], _path: &str, _opts: SaveWavOpts) -> PluginResult<()> {
        Err(PluginError::Custom(
            "audio.saveWav: rebuild with --features wav".into(),
        ))
    }
}

mod backend {
    pub(crate) use super::{
        capture_backend::{
            list_devices, play_file, play_samples, shutdown, start_capture, stop_capture,
            stop_playback,
        },
        wav_backend::save_wav,
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_validation() {
        assert!(validate_audio_format("t", 44_100, 2, 4).is_ok());
        assert!(validate_audio_format("t", 44_100, 2, 0).is_ok());
        assert!(validate_audio_format("t", 0, 2, 4).is_err());
        assert!(validate_audio_format("t", 44_100, 0, 4).is_err());
        assert!(validate_audio_format("t", 44_100, 2, 3).is_err());
    }

    #[test]
    fn full_playback_pool_errors_instead_of_blocking() {
        let plugin = AudioPlugin::default();
        let mut held = Vec::new();
        while let Some(slot) = plugin.playback_slots.try_acquire("t").unwrap() {
            held.push(slot);
        }
        assert!(plugin.acquire_playback_permits("t").is_err());
        held.pop();
        assert!(plugin.acquire_playback_permits("t").is_ok());
    }
}
