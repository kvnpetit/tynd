use tynd_plugin_sdk::{EmitFn, PluginError, PluginResult};

use super::{CaptureOpts, PlayFileOpts, PlaySamplesOpts, PlaybackSlot};

pub(crate) struct Devices {
    pub inputs: Vec<String>,
    pub outputs: Vec<String>,
}

pub(crate) fn list_devices() -> PluginResult<Devices> {
    Ok(Devices {
        inputs: Vec::new(),
        outputs: Vec::new(),
    })
}

pub(crate) fn start_capture(_id: u64, _opts: CaptureOpts, _emit: EmitFn) -> PluginResult<u64> {
    Err(PluginError::Custom(
        "audio.startCapture: rebuild with --features capture".into(),
    ))
}

pub(crate) fn stop_capture(_id: u64) -> PluginResult<()> {
    Ok(())
}

pub(crate) fn play_file(
    _id: u64,
    _path: &str,
    _opts: PlayFileOpts,
    _emit: EmitFn,
    _slot: PlaybackSlot,
) -> PluginResult<(u64, u64)> {
    Err(PluginError::Custom(
        "audio.playFile: rebuild with --features capture".into(),
    ))
}

pub(crate) fn play_samples(
    _id: u64,
    _samples: Vec<f32>,
    _opts: PlaySamplesOpts,
    _emit: EmitFn,
    _slot: PlaybackSlot,
) -> PluginResult<u64> {
    Err(PluginError::Custom(
        "audio.playSamples: rebuild with --features capture".into(),
    ))
}

pub(crate) fn stop_playback(_id: u64) -> PluginResult<()> {
    Ok(())
}

pub(crate) fn shutdown() -> PluginResult<()> {
    Ok(())
}
