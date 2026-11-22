use std::time::{Duration, Instant};

use tynd_plugin_sdk::{PluginError, PluginResult};

use crate::{Codec, MonitorInfo, RecordingOpts, Sources, WindowInfo};

/// Stub mirrors the real `Recorder` shape so call sites stay
/// `cfg`-free. Fields are dead in this build.
pub(crate) struct Recorder {
    pub(crate) started_at: Instant,
    /// Wall-clock epoch ms snapshot taken at `start()` — `list()`
    /// reads this back instead of subtracting an `Instant` from
    /// `SystemTime::now()` (mixed-clock drift).
    pub(crate) started_at_ms: i64,
    pub(crate) output: String,
    pub(crate) paused_for: Duration,
    pub(crate) paused_at: Option<Instant>,
}

pub(crate) fn stop(_recorder: &Recorder) -> PluginResult<()> {
    Ok(())
}

pub(crate) fn detach(_recorder: &Recorder) {}

pub(crate) fn failure(_recorder: &Recorder) -> Option<String> {
    None
}

pub(crate) fn start(_opts: RecordingOpts) -> PluginResult<Recorder> {
    Err(PluginError::Custom(
        "screen-rec: rebuild this plugin with `--features capture` to enable recording (the \
         `scap` native backend is opt-in to keep the default build slim)"
            .into(),
    ))
}

pub(crate) fn list_sources() -> PluginResult<Sources> {
    Ok(Sources {
        monitors: Vec::<MonitorInfo>::new(),
        windows: Vec::<WindowInfo>::new(),
    })
}

pub(crate) fn has_permission() -> bool {
    false
}

pub(crate) fn request_permission() -> bool {
    false
}

/// Claim MJPEG so dispatch falls through to `start()` for the
/// canonical "rebuild with --features capture" hint. H.264 / HEVC
/// short-circuit with the gpu-encode hint instead.
pub(crate) fn codec_supported(codec: Codec) -> bool {
    matches!(codec, Codec::Mjpeg)
}
