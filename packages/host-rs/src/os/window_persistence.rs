//! Window-state persistence — stores per-window size, position and
//! maximized / fullscreen flags between app sessions.
//!
//! The native module is a thin storage primitive: an in-memory
//! `HashMap<label, WindowState>` plus an atomic JSON file on disk.
//!
//! Capturing the live geometry (and applying it on restore) lives on
//! the TS client side because the `tao::window::Window` query helpers
//! are main-thread-only and not reachable from the background dispatch
//! that backs `os::dispatch`. The TS client subscribes to the existing
//! `window:resized` / `window:moved` / `window:maximized` events
//! (already emitted by `app/mod.rs`) and pushes snapshots through this
//! module's `set` + `saveToDisk` methods, mirroring the previous
//! `@tynd/plugin-window-state` design.

use std::{
    collections::HashMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::OnceLock,
};

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// One window's persisted state. Optional fields stay absent in the
/// JSON file when unknown — that way a window restored before the host
/// has reported its position doesn't get pinned to (0, 0).
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct WindowState {
    /// Outer-position X in logical pixels.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub x: Option<i32>,
    /// Outer-position Y in logical pixels.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub y: Option<i32>,
    /// Inner-size width in logical pixels.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub width: Option<u32>,
    /// Inner-size height in logical pixels.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub height: Option<u32>,
    /// Whether the window was maximized.
    #[serde(default)]
    pub maximized: bool,
    /// Whether the window was fullscreen.
    #[serde(default)]
    pub fullscreen: bool,
}

/// Process-wide store. Lazy-init so the cost is paid only when the API
/// is actually exercised (custom builds without `api-window-state` skip
/// it entirely via the feature gate).
struct Store {
    states: RwLock<HashMap<String, WindowState>>,
    storage_path: RwLock<Option<PathBuf>>,
}

impl Store {
    fn new() -> Self {
        Self {
            states: RwLock::new(HashMap::new()),
            storage_path: RwLock::new(None),
        }
    }
}

static STORE: OnceLock<Store> = OnceLock::new();

fn store() -> &'static Store {
    STORE.get_or_init(Store::new)
}

/// Replace the storage path. Public so embedders can pre-seed the path
/// before the TS layer ever calls in (rarely needed; the TS client
/// handles it via `windowState.setStoragePath` / the auto-default).
pub fn set_storage_path(path: PathBuf) {
    *store().storage_path.write() = Some(path);
}

/// Currently saved snapshot for `label`, if any.
pub fn get(label: &str) -> Option<WindowState> {
    store().states.read().get(label).cloned()
}

/// Drop one entry, or every entry when `label` is `None`.
pub fn clear(label: Option<&str>) -> std::io::Result<()> {
    let mut guard = store().states.write();
    match label {
        Some(l) => {
            guard.remove(l);
        },
        None => guard.clear(),
    }
    Ok(())
}

/// Apply a snapshot to the in-memory map. Does **not** flush to disk —
/// callers batch with `save_to_disk`.
pub fn set(label: &str, state: WindowState) {
    store().states.write().insert(label.to_string(), state);
}

/// Read the JSON file at `path` and replace the in-memory map.
/// Corrupted files fall through to an empty map (the TS client's
/// next snapshot will reseed). Sets the configured storage path as
/// a side-effect so subsequent saves are unambiguous.
pub fn load_from_disk(path: PathBuf) -> std::io::Result<()> {
    let map = read_states_or_empty(&path);
    *store().states.write() = map;
    *store().storage_path.write() = Some(path);
    Ok(())
}

/// Atomically write the in-memory map to `path` via a `*.tmp` +
/// rename dance. Creates the parent directory if it doesn't exist.
pub fn save_to_disk() -> std::io::Result<()> {
    let path = store()
        .storage_path
        .read()
        .clone()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "no storage path configured — call setStoragePath / loadFromDisk first"))?;
    let snapshot = store().states.read().clone();
    write_states_atomic(&path, &snapshot)
}

/// IPC dispatch entry point — wired into `os::dispatch` under the
/// `windowState` API name. Mirrors the wire shape the previous
/// `@tynd/plugin-window-state` plugin exposed so existing client
/// code continues to work after the rename.
pub fn dispatch(method: &str, args: &Value) -> Result<Value, String> {
    match method {
        "get" => dispatch_get(args),
        "getAll" => dispatch_get_all(),
        "set" => dispatch_set(args),
        "clear" => dispatch_clear(args),
        "setStoragePath" => dispatch_set_storage_path(args),
        "loadFromDisk" => dispatch_load_from_disk(args),
        "saveToDisk" => dispatch_save_to_disk(),
        _ => Err(format!("windowState.{method}: unknown method")),
    }
}

fn dispatch_get(args: &Value) -> Result<Value, String> {
    let label = args
        .get("label")
        .and_then(Value::as_str)
        .ok_or_else(|| "windowState.get: missing 'label'".to_string())?;
    Ok(get(label).map_or(Value::Null, |s| {
        serde_json::to_value(s).unwrap_or(Value::Null)
    }))
}

fn dispatch_get_all() -> Result<Value, String> {
    let guard = store().states.read();
    let mut out = serde_json::Map::with_capacity(guard.len());
    for (label, state) in guard.iter() {
        out.insert(
            label.clone(),
            serde_json::to_value(state).unwrap_or(Value::Null),
        );
    }
    Ok(Value::Object(out))
}

fn dispatch_set(args: &Value) -> Result<Value, String> {
    let label = args
        .get("label")
        .and_then(Value::as_str)
        .ok_or_else(|| "windowState.set: missing 'label'".to_string())?;
    let raw = args
        .get("state")
        .ok_or_else(|| "windowState.set: missing 'state'".to_string())?;
    let state: WindowState = serde_json::from_value(raw.clone())
        .map_err(|e| format!("windowState.set: invalid 'state': {e}"))?;
    set(label, state);
    Ok(Value::Null)
}

fn dispatch_clear(args: &Value) -> Result<Value, String> {
    let label = args.get("label").and_then(Value::as_str);
    clear(label).map_err(|e| format!("windowState.clear: {e}"))?;
    Ok(Value::Null)
}

fn dispatch_set_storage_path(args: &Value) -> Result<Value, String> {
    let path = args
        .get("path")
        .and_then(Value::as_str)
        .ok_or_else(|| "windowState.setStoragePath: missing 'path'".to_string())?;
    set_storage_path(PathBuf::from(path));
    Ok(Value::Null)
}

fn dispatch_load_from_disk(args: &Value) -> Result<Value, String> {
    // `path` is optional once `setStoragePath` has been called; fall
    // back to the configured path otherwise.
    let path = if let Some(p) = args.get("path").and_then(Value::as_str) {
        PathBuf::from(p)
    } else {
        store()
            .storage_path
            .read()
            .clone()
            .ok_or_else(|| "windowState.loadFromDisk: no path provided and none configured".to_string())?
    };
    load_from_disk(path).map_err(|e| format!("windowState.loadFromDisk: {e}"))?;
    Ok(Value::Null)
}

fn dispatch_save_to_disk() -> Result<Value, String> {
    save_to_disk().map_err(|e| format!("windowState.saveToDisk: {e}"))?;
    Ok(json!(null))
}

/// Read a JSON file and parse it into a label → state map. Returns an
/// empty map on any failure (file missing, IO error, malformed JSON,
/// wrong shape) — corrupted state should never crash the host.
fn read_states_or_empty(path: &Path) -> HashMap<String, WindowState> {
    let bytes = match fs::read(path) {
        Ok(b) => b,
        Err(_) => return HashMap::new(),
    };
    serde_json::from_slice::<HashMap<String, WindowState>>(&bytes).unwrap_or_default()
}

/// Atomic write via tempfile + rename. Creates the parent directory
/// if needed. The temp file lives in the same directory as the target
/// so the rename stays on a single filesystem.
fn write_states_atomic(
    path: &Path,
    states: &HashMap<String, WindowState>,
) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_vec_pretty(states)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    let tmp = path.with_extension("json.tmp");
    {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(&json)?;
        f.sync_all()?;
    }
    // `rename` is atomic on POSIX; on Windows it replaces the target
    // when the target exists (since Rust 1.84 the std impl uses
    // `MoveFileExW(MOVEFILE_REPLACE_EXISTING)`).
    fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use parking_lot::Mutex;

    use super::*;

    /// Tests run serially because the underlying store is process-wide.
    /// Without this lock, parallel tests would clobber each other's
    /// in-memory state and assert against the wrong baseline.
    fn test_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    fn reset() {
        let _ = clear(None);
        *store().storage_path.write() = None;
    }

    fn sample() -> Value {
        json!({
            "x": 100,
            "y": 200,
            "width": 1200_u32,
            "height": 800_u32,
            "maximized": false,
            "fullscreen": false,
        })
    }

    #[test]
    fn set_then_get_round_trips() {
        let _g = test_lock().lock();
        reset();
        dispatch("set", &json!({ "label": "main", "state": sample() })).unwrap();
        let got = dispatch("get", &json!({ "label": "main" })).unwrap();
        assert_eq!(got["x"].as_i64().unwrap(), 100);
        assert_eq!(got["y"].as_i64().unwrap(), 200);
        assert_eq!(got["width"].as_u64().unwrap(), 1200);
        assert_eq!(got["height"].as_u64().unwrap(), 800);
        assert_eq!(got["maximized"].as_bool().unwrap(), false);
    }

    #[test]
    fn get_unknown_label_returns_null() {
        let _g = test_lock().lock();
        reset();
        let v = dispatch("get", &json!({ "label": "missing" })).unwrap();
        assert!(v.is_null());
    }

    #[test]
    fn clear_one_label_keeps_others() {
        let _g = test_lock().lock();
        reset();
        dispatch("set", &json!({ "label": "main", "state": sample() })).unwrap();
        dispatch(
            "set",
            &json!({ "label": "settings", "state": sample() }),
        )
        .unwrap();
        dispatch("clear", &json!({ "label": "main" })).unwrap();
        assert!(dispatch("get", &json!({ "label": "main" }))
            .unwrap()
            .is_null());
        assert!(!dispatch("get", &json!({ "label": "settings" }))
            .unwrap()
            .is_null());
    }

    #[test]
    fn clear_without_label_drops_everything() {
        let _g = test_lock().lock();
        reset();
        dispatch("set", &json!({ "label": "main", "state": sample() })).unwrap();
        dispatch("clear", &json!({})).unwrap();
        let all = dispatch("getAll", &json!(null)).unwrap();
        assert_eq!(all, json!({}));
    }

    #[test]
    fn save_then_load_round_trips_through_disk() {
        let _g = test_lock().lock();
        reset();
        let dir = std::env::temp_dir().join(format!(
            "tynd-window-persistence-test-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        let path = dir.join("window-states.json");

        dispatch(
            "setStoragePath",
            &json!({ "path": path.to_string_lossy().to_string() }),
        )
        .unwrap();
        dispatch("set", &json!({ "label": "main", "state": sample() })).unwrap();
        dispatch("saveToDisk", &json!(null)).unwrap();

        // Wipe in-memory + reload from disk; should repopulate.
        clear(None).unwrap();
        dispatch(
            "loadFromDisk",
            &json!({ "path": path.to_string_lossy().to_string() }),
        )
        .unwrap();
        let got = dispatch("get", &json!({ "label": "main" })).unwrap();
        assert_eq!(got["x"].as_i64().unwrap(), 100);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_from_corrupted_file_is_empty_not_error() {
        let _g = test_lock().lock();
        reset();
        let dir = std::env::temp_dir().join(format!(
            "tynd-window-persistence-test-corrupt-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("window-states.json");
        fs::write(&path, b"{this is not valid json").unwrap();

        let res = dispatch(
            "loadFromDisk",
            &json!({ "path": path.to_string_lossy().to_string() }),
        );
        assert!(res.is_ok(), "corrupted JSON should not surface as Err");
        let all = dispatch("getAll", &json!(null)).unwrap();
        assert_eq!(all, json!({}));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_without_storage_path_errors_clearly() {
        let _g = test_lock().lock();
        reset();
        let err = dispatch("saveToDisk", &json!(null)).unwrap_err();
        assert!(err.contains("no storage path"));
    }

    #[test]
    fn missing_label_on_get_errors() {
        let _g = test_lock().lock();
        reset();
        let err = dispatch("get", &json!({})).unwrap_err();
        assert!(err.contains("missing 'label'"));
    }

    #[test]
    fn unknown_method_errors() {
        let _g = test_lock().lock();
        reset();
        let err = dispatch("nope", &json!({})).unwrap_err();
        assert!(err.contains("unknown method"));
    }
}
