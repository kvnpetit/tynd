#[cfg(not(feature = "capture"))]
mod disabled;
#[cfg(feature = "capture")]
mod enabled;

#[cfg(not(feature = "capture"))]
pub(crate) use disabled::{
    codec_supported, detach, failure, has_permission, list_sources, request_permission, start,
    stop, Recorder,
};
#[cfg(feature = "capture")]
pub(crate) use enabled::{
    codec_supported, detach, failure, has_permission, list_sources, request_permission, start,
    stop, Recorder,
};
