//! Structured logger with size-based rotation. JSON-lines on disk for ops tooling.
//!
//! Promoted from `@tynd/plugin-log` (commit 74dc253 template). The semantics
//! match the original plugin verbatim — `configure` / `write` / `path` —
//! but the dispatch is sync and routes through `os::dispatch` instead of
//! `crate::plugins::dispatch`. File logging is opt-in: callers must call
//! `log.configure({...})` (or `os::log::init_from_config` from the host)
//! before the first `write`. The default level is `info`.

use std::{
    io::Write as _,
    path::PathBuf,
    sync::LazyLock,
    time::{SystemTime, UNIX_EPOCH},
};

// `fs-err` shim — the rotation helper deliberately uses `let _ =` for
// best-effort cleanup, but `ensure_open` does propagate errors and
// hand-stamps `format!("log: open {}: {e}", path.display())`. Routing
// through `fs_err` collapses that to `format!("log: {e}")`.
use fs_err as fs;
use fs_err::{File, OpenOptions};

use parking_lot::Mutex;
use serde_json::{json, Value};

use crate::ipc_error::IpcError;

const DEFAULT_MAX_BYTES: u64 = 5 * 1024 * 1024;
const DEFAULT_MAX_FILES: u32 = 3;

/// File-rotation strategy. Today the only canonical strategy is
/// `Size` (the default — preserves the original `os::log` semantics
/// every shipping app already pins against). `Daily`/`Hourly` defer
/// to `tracing-appender`'s `RollingFileAppender` for time-based
/// rotation; `Both` cuts on whichever criterion fires first.
///
/// Wiring is mostly stub for V1.0: the JS-side config field is
/// accepted (no error on unknown values) and `Daily`/`Hourly`
/// degrade to `Size` if the appender wiring isn't reachable from the
/// `os::log::write` hot path. Full tracing-appender adoption lands
/// in a dedicated post-V1.0 pass — V1.0 only commits to the
/// **shape**: an extra `rotation` field today buys forward-compat
/// without a breaking config change later.
#[derive(Copy, Clone, PartialEq, Eq)]
enum Rotation {
    Size,
    Daily,
    Hourly,
    Both,
}

impl Rotation {
    fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "size" => Self::Size,
            "daily" => Self::Daily,
            "hourly" => Self::Hourly,
            "both" => Self::Both,
            _ => return None,
        })
    }
}

struct LoggerState {
    file: Option<File>,
    path: PathBuf,
    level: Level,
    max_bytes: u64,
    max_files: u32,
    bytes_written: u64,
    rotation: Rotation,
    /// Stamp of the last rotated time-bucket (Unix epoch seconds /
    /// 3600 for hourly, / 86400 for daily). Used by the
    /// time-rotation branches to decide if the current write should
    /// trigger a fresh rotation. `0` = unset; the next write will
    /// seed it without rotating.
    last_time_bucket: u64,
}

#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Level {
    Debug,
    Info,
    Warn,
    Error,
}

impl Level {
    fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "debug" => Self::Debug,
            "info" => Self::Info,
            "warn" => Self::Warn,
            "error" => Self::Error,
            _ => return None,
        })
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Debug => "debug",
            Self::Info => "info",
            Self::Warn => "warn",
            Self::Error => "error",
        }
    }
}

/// Process-wide logger state. `LazyLock` (Rust 1.80+) builds the inner
/// `Mutex<LoggerState>` on the first deref — no `get_or_init` accessor
/// boilerplate. Direct `STATE.lock()` calls now work everywhere the
/// old `STATE.lock()` did.
static STATE: LazyLock<Mutex<LoggerState>> = LazyLock::new(|| {
    Mutex::new(LoggerState {
        file: None,
        path: default_path(),
        level: Level::Info,
        max_bytes: DEFAULT_MAX_BYTES,
        max_files: DEFAULT_MAX_FILES,
        bytes_written: 0,
        // Size-based stays the default — every existing app pins
        // against `app.log` + `app.log.1` etc., so flipping the
        // default would break their rotation expectations.
        rotation: Rotation::Size,
        last_time_bucket: 0,
    })
});

fn default_path() -> PathBuf {
    let name = std::env::current_exe()
        .ok()
        .and_then(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "tynd".into());
    let base = dirs::cache_dir()
        .or_else(dirs::data_dir)
        .unwrap_or_else(std::env::temp_dir);
    base.join(&name).join("app.log")
}

pub fn dispatch(method: &str, args: &Value) -> Result<Value, String> {
    match method {
        "configure" => configure(args),
        "write" => write_entry(args),
        "path" => Ok(Value::String(
            STATE.lock().path.to_string_lossy().into_owned(),
        )),
        _ => Err(IpcError::unknown_method("log", method).into_string()),
    }
}

fn configure(args: &Value) -> Result<Value, String> {
    let mut s = STATE.lock();
    if let Some(level) = args.get("level").and_then(Value::as_str) {
        s.level = Level::parse(level).ok_or_else(|| {
            IpcError::new("log", "configure", format!("invalid level '{level}'")).into_string()
        })?;
    }
    if let Some(file) = args.get("file").and_then(Value::as_str) {
        let p = PathBuf::from(file);
        if !p.is_absolute() {
            return Err(
                IpcError::new("log", "configure", "'file' must be an absolute path").into_string(),
            );
        }
        s.path = p;
        s.file = None;
        s.bytes_written = 0;
    }
    if let Some(n) = args.get("maxBytes").and_then(Value::as_u64) {
        s.max_bytes = n.max(1024);
    }
    if let Some(n) = args.get("maxFiles").and_then(Value::as_u64) {
        s.max_files = u32::try_from(n).unwrap_or(DEFAULT_MAX_FILES).max(1);
    }
    if let Some(rot) = args.get("rotation").and_then(Value::as_str) {
        s.rotation = Rotation::parse(rot).ok_or_else(|| {
            IpcError::new("log", "configure", format!("invalid rotation '{rot}'")).into_string()
        })?;
        // Reset the time-bucket so the next write seeds it against
        // the new rotation cadence.
        s.last_time_bucket = 0;
    }
    Ok(Value::Null)
}

/// Compute the current time-bucket index for the rotation strategy.
/// `0` for `Size` (unused); seconds-since-epoch / `bucket_secs` otherwise.
fn current_time_bucket(rotation: Rotation) -> u64 {
    let bucket_secs: u64 = match rotation {
        Rotation::Hourly | Rotation::Both => 3600,
        Rotation::Daily => 86400,
        Rotation::Size => return 0,
    };
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() / bucket_secs)
}

fn write_entry(args: &Value) -> Result<Value, String> {
    let level_str = args
        .get("level")
        .and_then(Value::as_str)
        .ok_or_else(|| String::from(IpcError::missing_arg("log", "write", "level")))?;
    let level = Level::parse(level_str).ok_or_else(|| {
        IpcError::new("log", "write", format!("invalid level '{level_str}'")).into_string()
    })?;

    let message = args
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let fields = args.get("fields").cloned().unwrap_or(Value::Null);

    let mut s = STATE.lock();
    if level < s.level {
        return Ok(Value::Null);
    }

    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64);

    let mut entry = json!({
        "ts": ts,
        "level": level.as_str(),
        "message": message,
    });
    if !fields.is_null() {
        if let Some(obj) = entry.as_object_mut() {
            obj.insert("fields".into(), fields);
        }
    }

    let mut line = serde_json::to_string(&entry).map_err(|e| {
        IpcError::new("log", "write", format!("serialize entry failed: {e}")).into_string()
    })?;
    line.push('\n');

    ensure_open(&mut s)?;
    let bytes = line.as_bytes();
    if let Some(f) = s.file.as_mut() {
        f.write_all(bytes)
            .map_err(|e| IpcError::new("log", "write", e).into_string())?;
    }
    s.bytes_written += bytes.len() as u64;

    // Rotation triggers — Size and Time are evaluated independently so
    // `Both` cuts on whichever fires first.
    let size_trigger = matches!(s.rotation, Rotation::Size | Rotation::Both)
        && s.bytes_written >= s.max_bytes;
    let time_trigger = if matches!(s.rotation, Rotation::Daily | Rotation::Hourly | Rotation::Both)
    {
        let now_bucket = current_time_bucket(s.rotation);
        if s.last_time_bucket == 0 {
            // First write under a time strategy seeds the bucket
            // without rotating — otherwise every fresh process would
            // rotate on its first log line.
            s.last_time_bucket = now_bucket;
            false
        } else if now_bucket != s.last_time_bucket {
            s.last_time_bucket = now_bucket;
            true
        } else {
            false
        }
    } else {
        false
    };

    if size_trigger || time_trigger {
        rotate(&mut s);
    }
    Ok(Value::Null)
}

fn ensure_open(s: &mut LoggerState) -> Result<(), String> {
    if s.file.is_some() {
        return Ok(());
    }
    if let Some(parent) = s.path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| IpcError::new("log", "ensure_open", e).into_string())?;
    }
    let f = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&s.path)
        .map_err(|e| IpcError::new("log", "ensure_open", e).into_string())?;
    s.bytes_written = f.metadata().map_or(0, |m| m.len());
    s.file = Some(f);
    Ok(())
}

/// Rename `app.log` -> `app.log.1`, shift older backups up to
/// `max_files - 1`, drop anything past that. Matches logrotate's
/// "copytruncate"-free semantics.
fn rotate(s: &mut LoggerState) {
    s.file = None;
    let base = s.path.clone();
    let max = s.max_files;

    let numbered = |n: u32| -> PathBuf {
        let mut p = base.clone();
        let name = p
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        p.set_file_name(format!("{name}.{n}"));
        p
    };

    let oldest = numbered(max.saturating_sub(1));
    if oldest.exists() {
        let _ = std::fs::remove_file(&oldest);
    }
    for n in (1..max.saturating_sub(1)).rev() {
        let src = numbered(n);
        if src.exists() {
            let _ = std::fs::rename(&src, numbered(n + 1));
        }
    }
    if base.exists() {
        let _ = std::fs::rename(&base, numbered(1));
    }
    s.bytes_written = 0;
}

#[cfg(test)]
mod tests {
    use std::{path::Path, sync::Mutex as StdMutex};

    use super::*;

    /// Tests in this module share the process-wide `STATE` (the public
    /// `dispatch()` is meant to be a singleton). `cargo test` runs tests
    /// concurrently within a binary, so we need an extra serialization
    /// layer to keep file paths + level + rotation state from racing
    /// across cases. Plain `std::sync::Mutex` is fine — never contended.
    static TEST_LOCK: StdMutex<()> = StdMutex::new(());

    fn fresh(dir: &Path) {
        let mut s = STATE.lock();
        s.file = None;
        s.path = dir.join("t.log");
        s.level = Level::Debug;
        s.max_bytes = DEFAULT_MAX_BYTES;
        s.max_files = DEFAULT_MAX_FILES;
        s.bytes_written = 0;
        s.rotation = Rotation::Size;
        s.last_time_bucket = 0;
    }

    fn tmpdir() -> PathBuf {
        let mut d = std::env::temp_dir();
        d.push(format!(
            "tynd-log-test-{}-{}",
            std::process::id(),
            fastrand::u64(..)
        ));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn write_then_read() {
        let _g = TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let dir = tmpdir();
        fresh(&dir);
        write_entry(&json!({ "level": "info", "message": "hi-write-then-read" })).unwrap();
        let body = std::fs::read_to_string(dir.join("t.log")).unwrap();
        assert!(body.contains("\"level\":\"info\""));
        assert!(body.contains("hi-write-then-read"));
    }

    #[test]
    fn level_filter_drops_below_threshold() {
        let _g = TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let dir = tmpdir();
        fresh(&dir);
        {
            let mut s = STATE.lock();
            s.level = Level::Warn;
        }
        write_entry(&json!({ "level": "info", "message": "skip-lvl-filter" })).unwrap();
        write_entry(&json!({ "level": "error", "message": "keep-lvl-filter" })).unwrap();
        let body = std::fs::read_to_string(dir.join("t.log")).unwrap();
        assert!(!body.contains("skip-lvl-filter"));
        assert!(body.contains("keep-lvl-filter"));
    }

    #[test]
    fn configure_accepts_rotation_strategies() {
        let _g = TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let dir = tmpdir();
        fresh(&dir);
        for r in ["size", "daily", "hourly", "both"] {
            configure(&json!({ "rotation": r })).expect(r);
        }
        let err = configure(&json!({ "rotation": "weekly" })).unwrap_err();
        assert!(err.contains("invalid rotation"), "got: {err}");
    }

    #[test]
    fn rotates_when_exceeding_max_bytes() {
        let _g = TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let dir = tmpdir();
        fresh(&dir);
        {
            let mut s = STATE.lock();
            s.max_bytes = 200;
            s.max_files = 3;
        }
        for i in 0..20 {
            write_entry(&json!({ "level": "info", "message": format!("entry-{i}") })).unwrap();
        }
        assert!(dir.join("t.log.1").exists());
    }
}
