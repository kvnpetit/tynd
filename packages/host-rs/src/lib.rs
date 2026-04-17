pub mod app;
pub mod cleanup;
pub mod ipc;
pub mod log;
pub(crate) mod menu;
pub mod os;
pub mod runtime;
pub mod scheme;
pub(crate) mod tray;
pub mod window;

/// Back-compat alias for `tynd_info!`. Prefer `tynd_info!` / `tynd_warn!` /
/// `tynd_error!` / `tynd_debug!` in new code.
#[macro_export]
macro_rules! tynd_log {
    ($($arg:tt)*) => { $crate::tynd_info!($($arg)*) };
}

/// Info-level framework log — always printed unless `TYND_LOG=warn|error|off`.
#[macro_export]
macro_rules! tynd_info {
    ($($arg:tt)*) => {
        if $crate::log::enabled($crate::log::Level::Info) {
            eprintln!("[tynd] {}", format!($($arg)*));
        }
    };
}

/// Warning-level framework log — on unless `TYND_LOG=error|off`.
#[macro_export]
macro_rules! tynd_warn {
    ($($arg:tt)*) => {
        if $crate::log::enabled($crate::log::Level::Warn) {
            eprintln!("[tynd][warn] {}", format!($($arg)*));
        }
    };
}

/// Error-level framework log — on unless `TYND_LOG=off`.
#[macro_export]
macro_rules! tynd_error {
    ($($arg:tt)*) => {
        if $crate::log::enabled($crate::log::Level::Error) {
            eprintln!("[tynd][error] {}", format!($($arg)*));
        }
    };
}

/// Debug-level framework log — only shown when `TYND_LOG=debug`.
#[macro_export]
macro_rules! tynd_debug {
    ($($arg:tt)*) => {
        if $crate::log::enabled($crate::log::Level::Debug) {
            eprintln!("[tynd][debug] {}", format!($($arg)*));
        }
    };
}
