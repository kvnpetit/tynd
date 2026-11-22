use std::{
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
};

use tynd_plugin_sdk::{PluginError, PluginResult};

use crate::{TranscribeOptions, TranscriptSegment};

const FEATURE_HINT: &str = "whisper: rebuild with --features inference to enable \
                            transcription (whisper.cpp must be available on the toolchain)";

pub(crate) struct WhisperModel {
    pub(crate) path: PathBuf,
}

/// Test-only constructor (the stub cannot load real models).
#[cfg(test)]
pub(crate) fn test_model(path: &str) -> WhisperModel {
    WhisperModel {
        path: PathBuf::from(path),
    }
}

pub(crate) fn load(path: &Path, gpu: bool) -> PluginResult<WhisperModel> {
    let _ = (path, gpu);
    Err(PluginError::Custom(FEATURE_HINT.into()))
}

pub(crate) fn transcribe<F>(
    model: &WhisperModel,
    samples: &[f32],
    options: &TranscribeOptions,
    cancel: &AtomicBool,
    callback: F,
) -> PluginResult<()>
where
    F: FnMut(TranscriptSegment) -> bool,
{
    let _ = (model, samples, options, cancel, callback);
    Err(PluginError::Custom(FEATURE_HINT.into()))
}

pub(crate) fn detect_language(
    model: &WhisperModel,
    samples: &[f32],
) -> PluginResult<(String, f32)> {
    let _ = (model, samples);
    Err(PluginError::Custom(FEATURE_HINT.into()))
}
