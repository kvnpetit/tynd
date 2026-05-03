//! Optional rotating-file sink for the host's `tracing` pipeline.
//!
//! `lib.rs::init_tracing` installs a `tracing_subscriber::fmt` layer
//! pointed at stderr — that's the right default for a desktop process
//! whose output the launcher captures into the OS log (`launchd` /
//! `journald` / Windows Event Log). Production deployments that want a
//! local on-disk audit trail call `setup_rotating_file` from their own
//! init path and compose the returned writer into the subscriber:
//!
//! ```ignore
//! let (writer, _guard) = tynd_host::logging::setup_rotating_file(
//!     std::path::Path::new("logs"),
//!     "app.log",
//!     "10 MiB",
//! )?;
//! // Keep `_guard` alive for the lifetime of the process — when it
//! // drops, the non-blocking worker thread shuts down and any
//! // buffered lines are flushed.
//! tracing_subscriber::fmt()
//!     .with_writer(writer)
//!     .with_ansi(false)
//!     .json()
//!     .init();
//! ```
//!
//! The size string is parsed with [`byte_unit`] so config files can
//! use the familiar `"10 MiB"` / `"500 KB"` / `"2 GiB"` notation
//! instead of raw byte counts. Note: today's `tracing-appender`
//! rotation is **time-based** (`hourly` / `daily` / `never`); the
//! parsed size is stored on the returned `RotationConfig` so
//! consumers that want size-based pruning (or a future
//! tracing-appender release that supports it) have the value at
//! hand without having to re-parse it.

use std::path::Path;

use tracing_appender::{
    non_blocking::{NonBlocking, WorkerGuard},
    rolling::{self, RollingFileAppender},
};

/// Inputs to [`setup_rotating_file`], also returned so callers can
/// log the resolved values for debugging.
#[derive(Debug)]
pub struct RotationConfig {
    /// Resolved max file size in bytes. Parsed from the human string
    /// (`"10 MiB"`); kept around so consumers can log it or feed it
    /// to a size-based pruner.
    pub max_bytes: u64,
}

/// Set up a daily-rotating, non-blocking file appender.
///
/// Returns `(NonBlocking, WorkerGuard, RotationConfig)`:
///
/// - The `NonBlocking` writer is what gets passed to
///   `tracing_subscriber::fmt().with_writer(...)`.
/// - The `WorkerGuard` **must** be kept alive for the lifetime of
///   the process; dropping it shuts the writer thread down and any
///   buffered lines get discarded. Stash it in a `static` or in
///   your top-level `App` struct.
/// - The `RotationConfig` echoes back the parsed `max_size` so
///   callers can log / surface it.
///
/// `max_size` accepts the standard human string format (`"10 MiB"`,
/// `"500 KB"`, `"2 GiB"`, `"1024"` for raw bytes). An invalid string
/// returns an error rather than silently defaulting — config
/// surprises in the log pipeline are exactly what you don't want
/// post-mortem.
pub fn setup_rotating_file(
    dir: &Path,
    file_name_prefix: &str,
    max_size: &str,
) -> Result<(NonBlocking, WorkerGuard, RotationConfig), String> {
    use byte_unit::Byte;

    let parsed = Byte::parse_str(max_size, true)
        .map_err(|e| format!("logging: invalid max_size '{max_size}' — {e}"))?;
    let max_bytes = parsed.as_u64();

    let appender: RollingFileAppender = rolling::daily(dir, file_name_prefix);
    let (writer, guard) = tracing_appender::non_blocking(appender);

    Ok((writer, guard, RotationConfig { max_bytes }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_human_size_and_creates_writer() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (_writer, _guard, cfg) =
            setup_rotating_file(dir.path(), "tynd-test.log", "10 MiB").expect("setup");
        assert_eq!(cfg.max_bytes, 10 * 1024 * 1024);
    }

    #[test]
    fn rejects_invalid_size_string() {
        let dir = tempfile::tempdir().expect("tempdir");
        let err = setup_rotating_file(dir.path(), "tynd-test.log", "not a size")
            .expect_err("should reject");
        assert!(err.contains("invalid max_size"));
    }

    #[test]
    fn accepts_raw_byte_count() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (_writer, _guard, cfg) =
            setup_rotating_file(dir.path(), "tynd-test.log", "1024").expect("setup");
        assert_eq!(cfg.max_bytes, 1024);
    }
}
