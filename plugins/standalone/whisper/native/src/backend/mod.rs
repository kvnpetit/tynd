#[cfg(not(feature = "inference"))]
mod disabled;
#[cfg(feature = "inference")]
mod enabled;

#[cfg(all(test, not(feature = "inference")))]
pub(crate) use disabled::test_model;
#[cfg(not(feature = "inference"))]
pub(crate) use disabled::{detect_language, load, transcribe, WhisperModel};
#[cfg(feature = "inference")]
pub(crate) use enabled::{detect_language, load, transcribe, WhisperModel};
