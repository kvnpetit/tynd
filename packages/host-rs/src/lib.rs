pub mod app;
pub mod cleanup;
pub mod ipc;
pub mod os;
pub mod runtime;
pub mod scheme;
pub mod window;

/// Log a developer-facing diagnostic message prefixed with `[vorn]`.
///
/// Use this for messages that help app developers diagnose issues with their
/// Vorn setup — wrong args, missing config, IPC errors, etc.
/// Do NOT use for messages that appear in shipped app binaries (e.g. first-launch
/// runtime extraction) where the framework name would confuse end users.
#[macro_export]
macro_rules! vorn_log {
    ($($arg:tt)*) => {
        eprintln!("[vorn] {}", format!($($arg)*))
    };
}
