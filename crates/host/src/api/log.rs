//! Structured JSON-lines logger with size and UTC time-bucket rotation.
//!
//! Configuration is optional: the first `write` opens the platform-default
//! path with an `info` threshold, 5 MiB size limit, and three backups.

use std::{
    fmt::Display,
    fs::{self, File, OpenOptions},
    io::Write as _,
    path::{Path, PathBuf},
    sync::LazyLock,
    time::{SystemTime, UNIX_EPOCH},
};

use parking_lot::Mutex;
use serde_json::{json, Value};

use crate::api::HostApi;
use crate::ipc::error::IpcError;
use crate::tynd;

const DEFAULT_MAX_BYTES: u64 = 5 * 1024 * 1024;
const DEFAULT_MAX_FILES: u32 = 3;

/// File-rotation strategy. `Both` combines size and daily UTC rotation.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
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

#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
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

/// Process-wide logger state, built on first use with the defaults.
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

/// `<app log dir>/app.log` (`crate::app_dirs`, the folder `os.appLogDir`
/// reports).
fn default_path() -> PathBuf {
    crate::app_dirs::app_dir(crate::app_dirs::AppDir::Log)
        .unwrap_or_else(std::env::temp_dir)
        .join("app.log")
}

/// Dispatch marker — logger state lives in `STATE`.
#[derive(Debug, Default)]
pub(crate) struct LogApi;

/// Row of the host API table (`api::HOST_APIS`).
pub(crate) const API: HostApi = HostApi::new("log", |method, args| dispatch(method, &args));

#[tynd::handler(api = "log")]
impl LogApi {
    /// Update the logger config in-place. Every field is optional —
    /// supply only the keys you want to change. `level` accepts
    /// `"debug" | "info" | "warn" | "error"`. `file` must be an
    /// absolute path. `rotation` accepts `"size" | "daily" | "hourly"
    /// | "both"`.
    pub fn configure(
        level: Option<String>,
        file: Option<String>,
        max_bytes: Option<u64>,
        max_files: Option<u64>,
        rotation: Option<String>,
    ) -> Result<(), String> {
        configure_impl(level, file, max_bytes, max_files, rotation)
    }

    /// Write a single log entry. `level` is required; `message`
    /// defaults to the empty string; `fields` is an optional JSON
    /// object spliced into the entry under the `fields` key.
    pub fn write(
        level: String,
        message: Option<String>,
        fields: Option<Value>,
    ) -> Result<(), String> {
        write_entry_at(
            &level,
            message.as_deref().unwrap_or_default(),
            fields.as_ref(),
            SystemTime::now(),
        )
    }

    /// Return the absolute path of the current log file.
    pub fn path() -> String {
        STATE.lock().path.to_string_lossy().into_owned()
    }
}

/// `log.configure: <msg>` error string.
fn configure_error(msg: impl Display) -> String {
    IpcError::new("log", "configure", msg).into_string()
}

fn configure_impl(
    level: Option<String>,
    file: Option<String>,
    max_bytes: Option<u64>,
    max_files: Option<u64>,
    rotation: Option<String>,
) -> Result<(), String> {
    // Validate the whole update before mutating process-wide state. A bad
    // trailing field must not leave earlier fields partially applied.
    let level = level
        .map(|level| {
            Level::parse(&level).ok_or_else(|| configure_error(format!("invalid level '{level}'")))
        })
        .transpose()?;
    let file = file.as_deref().map(resolve_log_file).transpose()?;
    let max_files = max_files
        .map(|raw| {
            u32::try_from(raw)
                .map_err(|_| configure_error("'maxFiles' exceeds the supported range"))
        })
        .transpose()?;
    let rotation = rotation
        .map(|rotation| {
            Rotation::parse(&rotation)
                .ok_or_else(|| configure_error(format!("invalid rotation '{rotation}'")))
        })
        .transpose()?;

    let mut s = STATE.lock();
    if let Some(level) = level {
        s.level = level;
    }
    if let Some(file) = file {
        s.path = file;
        s.file = None;
        s.bytes_written = 0;
        s.last_time_bucket = 0;
    }
    if let Some(max_bytes) = max_bytes {
        s.max_bytes = max_bytes.max(1024);
    }
    if let Some(max_files) = max_files {
        s.max_files = max_files.max(1);
    }
    if let Some(rotation) = rotation {
        s.rotation = rotation;
        s.last_time_bucket = 0;
    }
    Ok(())
}

/// Absolute `file` as resolved by the filesystem allow-list.
fn resolve_log_file(file: &str) -> Result<PathBuf, String> {
    if !Path::new(file).is_absolute() {
        return Err(configure_error("'file' must be an absolute path"));
    }
    crate::security::policy::authorize_fs_path(file).map_err(configure_error)
}

fn time_bucket(rotation: Rotation, time: SystemTime) -> Option<u64> {
    let bucket_secs: u64 = match rotation {
        Rotation::Hourly => 3600,
        Rotation::Daily | Rotation::Both => 86400,
        Rotation::Size => return None,
    };
    time.duration_since(UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_secs() / bucket_secs)
}

/// `log.write: <msg>` error string.
fn write_error(msg: impl Display) -> String {
    IpcError::new("log", "write", msg).into_string()
}

fn write_entry_at(
    level: &str,
    message: &str,
    fields: Option<&Value>,
    now: SystemTime,
) -> Result<(), String> {
    let level =
        Level::parse(level).ok_or_else(|| write_error(format!("invalid level '{level}'")))?;

    let mut s = STATE.lock();
    if level < s.level {
        return Ok(());
    }

    // Serialize only after the level filter: dropping below-threshold lines
    // must not allocate the payload.
    let line = format_entry(level, message, fields, now)?;
    let bytes = line.as_bytes();

    ensure_open(&mut s)?;
    rotate_on_new_time_bucket(&mut s, now)?;
    rotate_on_size(&mut s, bytes.len() as u64)?;

    if let Some(f) = s.file.as_mut() {
        f.write_all(bytes).map_err(write_error)?;
    }
    s.bytes_written += bytes.len() as u64;
    Ok(())
}

/// One JSON line (`{ ts, level, message, fields? }` plus `\n`). `fields` is
/// omitted when absent or `null`.
fn format_entry(
    level: Level,
    message: &str,
    fields: Option<&Value>,
    now: SystemTime,
) -> Result<String, String> {
    let ts = now.duration_since(UNIX_EPOCH).map_or(0, |duration| {
        u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
    });

    let mut entry = json!({
        "ts": ts,
        "level": level.as_str(),
        "message": message,
    });
    if let Some(fields) = fields.filter(|fields| !fields.is_null()) {
        if let Some(obj) = entry.as_object_mut() {
            obj.insert("fields".into(), fields.clone());
        }
    }

    let mut line = serde_json::to_string(&entry)
        .map_err(|e| write_error(format!("serialize entry failed: {e}")))?;
    line.push('\n');
    Ok(line)
}

/// Rotate before the first write in a new UTC time bucket. Seed from the
/// existing file's modification time so a process started after midnight
/// does not append today's entries to yesterday's file.
fn rotate_on_new_time_bucket(s: &mut LoggerState, now: SystemTime) -> Result<(), String> {
    let Some(now_bucket) = time_bucket(s.rotation, now) else {
        return Ok(());
    };
    if s.last_time_bucket == 0 {
        s.last_time_bucket = if s.bytes_written == 0 {
            now_bucket
        } else {
            s.file
                .as_ref()
                .and_then(|file| file.metadata().ok())
                .and_then(|metadata| metadata.modified().ok())
                .and_then(|modified| time_bucket(s.rotation, modified))
                .unwrap_or(now_bucket)
        };
    }
    if now_bucket > s.last_time_bucket {
        rotate(s);
        s.last_time_bucket = now_bucket;
        ensure_open(s)?;
    }
    Ok(())
}

/// Rotate before the entry that would cross the byte budget. Avoid
/// rotating an empty file when one oversized entry is written.
fn rotate_on_size(s: &mut LoggerState, entry_len: u64) -> Result<(), String> {
    let size_trigger = matches!(s.rotation, Rotation::Size | Rotation::Both)
        && s.bytes_written > 0
        && s.bytes_written.saturating_add(entry_len) > s.max_bytes;
    if size_trigger {
        rotate(s);
        ensure_open(s)?;
    }
    Ok(())
}

fn ensure_open(s: &mut LoggerState) -> Result<(), String> {
    if s.file.is_some() {
        return Ok(());
    }
    let path = s
        .path
        .to_str()
        .ok_or_else(|| open_error("configured path is not valid UTF-8"))?;
    let resolved = crate::security::policy::authorize_fs_path(path).map_err(open_error)?;
    s.path = resolved;
    if let Some(parent) = s.path.parent() {
        fs::create_dir_all(parent).map_err(open_error)?;
    }
    let f = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&s.path)
        .map_err(open_error)?;
    s.bytes_written = f.metadata().map_or(0, |m| m.len());
    s.file = Some(f);
    Ok(())
}

/// `log.ensure_open: <msg>` error string.
fn open_error(msg: impl Display) -> String {
    IpcError::new("log", "ensure_open", msg).into_string()
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

    let oldest = numbered(max);
    if oldest.exists() {
        if let Err(err) = fs::remove_file(&oldest) {
            tracing::warn!(
                "log rotation: remove {} failed: {err}",
                file_name_of(&oldest)
            );
        }
    }
    let mut rename_failed = false;
    for n in (1..max).rev() {
        let src = numbered(n);
        if src.exists() && !rename_logged(&src, &numbered(n + 1)) {
            rename_failed = true;
        }
    }
    if base.exists() && !rename_logged(&base, &numbered(1)) {
        rename_failed = true;
    }
    // Only reset the byte counter if the active log was actually rotated
    // away. Resetting on a failed rename would let the next write think
    // it's starting fresh and append to the existing file mid-record,
    // corrupting the tail.
    if !rename_failed {
        s.bytes_written = 0;
    }
}

/// Rename `src` to `dst`, logging a warning on failure. Returns whether the
/// rename succeeded.
fn rename_logged(src: &Path, dst: &Path) -> bool {
    match fs::rename(src, dst) {
        Ok(()) => true,
        Err(err) => {
            tracing::warn!(
                "log rotation: rename {} -> {} failed: {err}",
                file_name_of(src),
                file_name_of(dst)
            );
            false
        },
    }
}

/// File name only, so log lines in a customer's support upload don't leak
/// the absolute log directory layout.
fn file_name_of(p: &Path) -> String {
    p.file_name()
        .and_then(|n| n.to_str())
        .map_or_else(|| "?".to_string(), str::to_string)
}
