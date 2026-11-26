//! Local crash reports (`app.crashReporter` in the backend).
//!
//! Off until the backend calls `crashReporter.start()`, which names the report
//! directory. From then on the runtime writes one JSON file per crash:
//!
//! - `host-panic-*.json`: a panic that escaped the native host (message,
//!   location, backtrace);
//! - `backend-exit-*.json`: the Bun backend died on its own (exit code and the
//!   last stderr lines, where an uncaught exception's stack ends up).
//!
//! Nothing is uploaded: the app reads the reports on its next start
//! (`getCrashReports()`) and decides what to send where.

use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    sync::{LazyLock, Mutex, PoisonError},
    time::{SystemTime, UNIX_EPOCH},
};

use serde_json::{json, Value};

/// Stderr lines kept for a backend-exit report.
const STDERR_TAIL_LINES: usize = 50;
/// Longest stderr line kept (bytes); longer lines are cut.
const STDERR_LINE_MAX: usize = 2_048;
/// Longest accepted `directory` / `appVersion` argument.
const MAX_ARG_LEN: usize = 1_024;

#[derive(Default)]
struct State {
    directory: Option<PathBuf>,
    app_version: Option<String>,
}

static STATE: LazyLock<Mutex<State>> = LazyLock::new(Mutex::default);
static STDERR_TAIL: LazyLock<Mutex<VecDeque<String>>> = LazyLock::new(Mutex::default);
/// Last panic seen by the hook: written out only if it escapes `run()`, so a
/// panic a plugin call catches never produces a crash report.
static LAST_PANIC: LazyLock<Mutex<Option<Value>>> = LazyLock::new(Mutex::default);

fn state() -> std::sync::MutexGuard<'static, State> {
    STATE.lock().unwrap_or_else(PoisonError::into_inner)
}

/// `crashReporter.start({ directory?, appVersion? })` from the backend.
/// Creates the directory and answers `{ directory }`.
pub(crate) fn start(args: &Value) -> Result<Value, String> {
    let directory = match args.get("directory") {
        None | Some(Value::Null) => default_directory()?,
        Some(Value::String(raw)) => parse_directory(raw)?,
        Some(_) => return Err("crashReporter.start: directory must be a string".into()),
    };
    let app_version = match args.get("appVersion") {
        None | Some(Value::Null) => None,
        Some(Value::String(version)) if !version.is_empty() && version.len() <= MAX_ARG_LEN => {
            Some(version.clone())
        },
        Some(_) => {
            return Err(format!(
                "crashReporter.start: appVersion must be a string of 1..={MAX_ARG_LEN} bytes"
            ))
        },
    };
    std::fs::create_dir_all(&directory).map_err(|error| {
        format!(
            "crashReporter.start: cannot create {}: {error}",
            directory.display()
        )
    })?;
    let answer = json!({ "directory": directory.to_string_lossy() });
    let mut state = state();
    state.directory = Some(directory);
    state.app_version = app_version;
    Ok(answer)
}

fn parse_directory(raw: &str) -> Result<PathBuf, String> {
    if raw.is_empty() || raw.len() > MAX_ARG_LEN || raw.contains('\0') {
        return Err(format!(
            "crashReporter.start: directory must be a path of 1..={MAX_ARG_LEN} bytes"
        ));
    }
    let path = PathBuf::from(raw);
    if !path.is_absolute() {
        return Err(format!(
            "crashReporter.start: directory must be an absolute path, got '{raw}'"
        ));
    }
    Ok(path)
}

/// `<local data dir>/<app>/crashes`, resolved by the owner of every app
/// directory ([`tynd_host::app_dirs::crash_dir`]).
fn default_directory() -> Result<PathBuf, String> {
    tynd_host::app_dirs::crash_dir().ok_or_else(|| {
        "crashReporter.start: no local data directory on this system; pass `directory`".to_owned()
    })
}

/// Remember one backend stderr line for a later backend-exit report.
pub(crate) fn record_stderr_line(line: &str) {
    let mut tail = STDERR_TAIL.lock().unwrap_or_else(PoisonError::into_inner);
    if tail.len() == STDERR_TAIL_LINES {
        tail.pop_front();
    }
    tail.push_back(truncate(line, STDERR_LINE_MAX));
}

fn truncate(line: &str, max: usize) -> String {
    if line.len() <= max {
        return line.to_owned();
    }
    let mut end = max;
    while !line.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &line[..end])
}

/// Chain a panic hook that remembers the panic for [`write_escaped_panic`].
pub(crate) fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let message = panic_payload_message(info.payload());
        let location = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()));
        let backtrace = std::backtrace::Backtrace::force_capture().to_string();
        *LAST_PANIC.lock().unwrap_or_else(PoisonError::into_inner) = Some(json!({
            "message": message,
            "location": location,
            "backtrace": backtrace,
            "thread": std::thread::current().name().unwrap_or("<unnamed>"),
        }));
        previous(info);
    }));
}

fn panic_payload_message(payload: &(dyn std::any::Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|s| (*s).to_owned())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "<non-string panic>".to_owned())
}

/// A panic escaped the host: write its report (no-op unless started).
pub(crate) fn write_escaped_panic(payload: &(dyn std::any::Any + Send)) {
    let recorded = LAST_PANIC
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .take();
    let mut details =
        recorded.unwrap_or_else(|| json!({ "message": panic_payload_message(payload) }));
    if let Some(map) = details.as_object_mut() {
        map.insert("kind".into(), json!("host-panic"));
    }
    write_report("host-panic", details);
}

/// The backend exited on its own: write its report (no-op unless started).
/// `exit_code` is `None` when the process was killed by a signal or could not
/// be reaped.
pub(crate) fn write_backend_exit(exit_code: Option<i32>) {
    let stderr: Vec<String> = STDERR_TAIL
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .iter()
        .cloned()
        .collect();
    let message = exit_code.map_or_else(
        || "backend runtime was terminated".to_owned(),
        |code| format!("backend runtime exited with code {code}"),
    );
    write_report(
        "backend-exit",
        json!({
            "kind": "backend-exit",
            "message": message,
            "exitCode": exit_code,
            "stderr": stderr,
        }),
    );
}

fn write_report(kind: &str, details: Value) {
    let (directory, app_version) = {
        let state = state();
        let Some(directory) = state.directory.clone() else {
            return;
        };
        (directory, state.app_version.clone())
    };
    let time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| {
            u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
        });
    let report = with_environment(details, time, app_version.as_deref());
    if let Err(error) = write_file(&directory, kind, time, &report) {
        tynd_host::tynd_error!("crash report not written: {error}");
    }
}

fn with_environment(mut details: Value, time: u64, app_version: Option<&str>) -> Value {
    if let Some(map) = details.as_object_mut() {
        map.insert("time".into(), json!(time));
        map.insert("appVersion".into(), json!(app_version));
        map.insert("tyndVersion".into(), json!(env!("CARGO_PKG_VERSION")));
        map.insert("os".into(), json!(std::env::consts::OS));
        map.insert("arch".into(), json!(std::env::consts::ARCH));
    }
    details
}

fn write_file(directory: &Path, kind: &str, time: u64, report: &Value) -> Result<PathBuf, String> {
    let path = directory.join(format!("{kind}-{time}-{}.json", std::process::id()));
    let body = serde_json::to_vec_pretty(report).map_err(|error| error.to_string())?;
    std::fs::write(&path, body).map_err(|error| format!("{}: {error}", path.display()))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn directory_must_be_absolute_and_bounded() {
        assert!(parse_directory("crashes").is_err());
        assert!(parse_directory("").is_err());
        assert!(parse_directory(&"a".repeat(MAX_ARG_LEN + 1)).is_err());
        let absolute = std::env::temp_dir().join("tynd-crash-test");
        assert_eq!(
            parse_directory(&absolute.to_string_lossy()).unwrap(),
            absolute
        );
    }

    #[test]
    fn start_rejects_bad_argument_types() {
        assert!(start(&json!({ "directory": 1 })).is_err());
        assert!(start(&json!({ "directory": "relative/dir" })).is_err());
        let dir = std::env::temp_dir().join("tynd-crash-args");
        assert!(start(&json!({ "directory": dir.to_string_lossy(), "appVersion": 3 })).is_err());
    }

    #[test]
    fn stderr_tail_keeps_the_last_lines_and_cuts_long_ones() {
        for i in 0..(STDERR_TAIL_LINES + 5) {
            record_stderr_line(&format!("line {i}"));
        }
        record_stderr_line(&"é".repeat(STDERR_LINE_MAX));
        let tail = STDERR_TAIL.lock().unwrap();
        assert_eq!(tail.len(), STDERR_TAIL_LINES);
        assert_eq!(tail.front().map(String::as_str), Some("line 6"));
        let last = tail.back().unwrap();
        assert!(last.len() <= STDERR_LINE_MAX + '…'.len_utf8());
        assert!(last.ends_with('…'));
    }

    #[test]
    fn report_file_carries_the_environment() {
        let dir = std::env::temp_dir().join(format!("tynd-crash-file-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let report = with_environment(json!({ "kind": "backend-exit" }), 42, Some("1.2.3"));
        let path = write_file(&dir, "backend-exit", 42, &report).unwrap();
        let read: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(read["kind"], "backend-exit");
        assert_eq!(read["time"], 42);
        assert_eq!(read["appVersion"], "1.2.3");
        assert_eq!(read["os"], std::env::consts::OS);
        assert!(path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("backend-exit-42-"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
