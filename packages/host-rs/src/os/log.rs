//! Structured logger with size-based rotation. JSON-lines on disk so ops
//! tooling can parse without a custom format. Shared between backend and
//! frontend — both call `log.write` through the usual IPC path.

use parking_lot::Mutex;
use serde_json::{json, Value};
use std::fs::{File, OpenOptions};
use std::io::Write as _;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

const DEFAULT_MAX_BYTES: u64 = 5 * 1024 * 1024;
const DEFAULT_MAX_FILES: u32 = 3;

struct LoggerState {
    file: Option<File>,
    path: PathBuf,
    level: Level,
    max_bytes: u64,
    max_files: u32,
    bytes_written: u64,
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

static STATE: OnceLock<Mutex<LoggerState>> = OnceLock::new();

fn state() -> &'static Mutex<LoggerState> {
    STATE.get_or_init(|| {
        let path = default_path();
        Mutex::new(LoggerState {
            file: None,
            path,
            level: Level::Info,
            max_bytes: DEFAULT_MAX_BYTES,
            max_files: DEFAULT_MAX_FILES,
            bytes_written: 0,
        })
    })
}

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
        "path" => {
            let s = state().lock();
            Ok(Value::String(s.path.to_string_lossy().into_owned()))
        },
        _ => Err(format!("log.{method}: unknown method")),
    }
}

fn configure(args: &Value) -> Result<Value, String> {
    let mut s = state().lock();
    if let Some(level) = args.get("level").and_then(Value::as_str) {
        s.level =
            Level::parse(level).ok_or_else(|| format!("log.configure: invalid level '{level}'"))?;
    }
    if let Some(file) = args.get("file").and_then(Value::as_str) {
        s.path = PathBuf::from(file);
        s.file = None;
        s.bytes_written = 0;
    }
    if let Some(n) = args.get("maxBytes").and_then(Value::as_u64) {
        s.max_bytes = n.max(1024);
    }
    if let Some(n) = args.get("maxFiles").and_then(Value::as_u64) {
        s.max_files = u32::try_from(n).unwrap_or(DEFAULT_MAX_FILES).max(1);
    }
    Ok(Value::Null)
}

fn write_entry(args: &Value) -> Result<Value, String> {
    let level_str = args
        .get("level")
        .and_then(Value::as_str)
        .ok_or_else(|| "log.write: missing 'level'".to_string())?;
    let level =
        Level::parse(level_str).ok_or_else(|| format!("log.write: invalid level '{level_str}'"))?;

    let message = args.get("message").and_then(Value::as_str).unwrap_or("");
    let fields = args.get("fields").cloned().unwrap_or(Value::Null);

    let mut s = state().lock();
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

    let mut line = serde_json::to_string(&entry).map_err(|e| e.to_string())?;
    line.push('\n');

    ensure_open(&mut s)?;
    let bytes = line.as_bytes();
    if let Some(f) = s.file.as_mut() {
        f.write_all(bytes).map_err(|e| format!("log.write: {e}"))?;
    }
    s.bytes_written += bytes.len() as u64;

    if s.bytes_written >= s.max_bytes {
        rotate(&mut s);
    }
    Ok(Value::Null)
}

fn ensure_open(s: &mut LoggerState) -> Result<(), String> {
    if s.file.is_some() {
        return Ok(());
    }
    if let Some(parent) = s.path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("log: mkdir {e}"))?;
    }
    let f = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&s.path)
        .map_err(|e| format!("log: open {}: {e}", s.path.display()))?;
    s.bytes_written = f.metadata().map_or(0, |m| m.len());
    s.file = Some(f);
    Ok(())
}

/// Rename `app.log` -> `app.log.1`, shift older backups up to `max_files - 1`,
/// drop anything past that. Matches logrotate's "copytruncate"-free semantics.
fn rotate(s: &mut LoggerState) {
    s.file = None;
    let base = &s.path;
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
        let _ = std::fs::rename(base, numbered(1));
    }
    s.bytes_written = 0;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use tempfile::tempdir;

    // Tests share the global logger state — serialize them to avoid races.
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    fn fresh_state(dir: &std::path::Path) {
        let mut s = state().lock();
        s.file = None;
        s.path = dir.join("t.log");
        s.level = Level::Debug;
        s.max_bytes = DEFAULT_MAX_BYTES;
        s.max_files = DEFAULT_MAX_FILES;
        s.bytes_written = 0;
    }

    #[test]
    fn write_then_read() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempdir().unwrap();
        fresh_state(dir.path());
        write_entry(&json!({ "level": "info", "message": "hi" })).unwrap();
        let body = std::fs::read_to_string(dir.path().join("t.log")).unwrap();
        assert!(body.contains("\"level\":\"info\""));
        assert!(body.contains("\"message\":\"hi\""));
    }

    #[test]
    fn level_filter_drops_below_threshold() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempdir().unwrap();
        fresh_state(dir.path());
        {
            let mut s = state().lock();
            s.level = Level::Warn;
        }
        write_entry(&json!({ "level": "info", "message": "skip" })).unwrap();
        write_entry(&json!({ "level": "error", "message": "keep" })).unwrap();
        let body = std::fs::read_to_string(dir.path().join("t.log")).unwrap();
        assert!(!body.contains("skip"));
        assert!(body.contains("keep"));
    }

    #[test]
    fn rotates_when_exceeding_max_bytes() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempdir().unwrap();
        fresh_state(dir.path());
        {
            let mut s = state().lock();
            s.max_bytes = 200;
            s.max_files = 3;
        }
        for i in 0..20 {
            write_entry(&json!({ "level": "info", "message": format!("entry-{i}") })).unwrap();
        }
        assert!(dir.path().join("t.log.1").exists());
    }
}
