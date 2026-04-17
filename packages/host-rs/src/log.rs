//! Level-gated logging controlled by the `TYND_LOG` env var.
//!
//! Values (case-insensitive): `debug`, `info` (default), `warn`, `error`,
//! `off`. Anything else is treated as `info` so a typo doesn't silence logs.

use std::sync::OnceLock;

#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Level {
    Error = 0,
    Warn = 1,
    Info = 2,
    Debug = 3,
}

fn threshold() -> Level {
    static T: OnceLock<Level> = OnceLock::new();
    *T.get_or_init(|| {
        match std::env::var("TYND_LOG")
            .ok()
            .map(|v| v.to_ascii_lowercase())
            .as_deref()
        {
            Some("debug" | "trace") => Level::Debug,
            Some("warn") => Level::Warn,
            // "off" is handled separately by is_off(); any error-or-lower
            // threshold here is just a safe fallback.
            Some("error" | "off") => Level::Error,
            _ => Level::Info,
        }
    })
}

fn is_off() -> bool {
    static OFF: OnceLock<bool> = OnceLock::new();
    *OFF.get_or_init(|| {
        std::env::var("TYND_LOG")
            .ok()
            .is_some_and(|v| v.eq_ignore_ascii_case("off"))
    })
}

/// Is this level enabled by the current `TYND_LOG` setting?
pub fn enabled(level: Level) -> bool {
    if is_off() {
        return false;
    }
    level <= threshold()
}
