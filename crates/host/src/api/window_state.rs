//! Window-state persistence — stores per-window size, position and
//! maximized / fullscreen flags between app sessions.
//!
//! The native module is a thin storage primitive: an in-memory
//! `HashMap<label, WindowState>` (the hot path, served from `DashMap`)
//! plus a `redb` database file on disk for durability.
//!
//! Capturing the live geometry (and applying it on restore) lives on
//! the TS client side because the `tao::window::Window` query helpers
//! are main-thread-only and not reachable from the background dispatch
//! that backs `api::dispatch`. The TS client subscribes to the existing
//! `window:resized` / `window:moved` / `window:maximized` events
//! (already emitted by `app.rs`) and pushes snapshots through this
//! module's `set` + `saveToDisk` methods.
//!
//! Storage path semantics: callers pass a path such as
//! `window-states.json`; the database file is the same path with a `.redb`
//! extension.

use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::OnceLock,
};

use dashmap::DashMap;
use parking_lot::RwLock;
use redb::{Database, ReadableDatabase, ReadableTable, TableDefinition};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::api::HostApi;
use crate::ipc::error::IpcError;
use crate::tynd;

/// Concurrent map for the window-state cache. DashMap is already used
/// throughout the host and is sufficient for this low-cardinality map.
type StateMap = DashMap<String, WindowState>;

/// Single redb table — keyed by window label, value is a JSON-encoded
/// `WindowState`. We keep the value column on serde_json (rather than
/// pulling postcard/bincode) because: (a) no compact binary format is
/// already in the workspace dep graph, (b) windows-per-app is < 10 so
/// payload size doesn't matter, (c) the schema-evolution story stays
/// trivial — `#[serde(skip_serializing_if = "Option::is_none")]` on
/// optional fields keeps old rows readable across version bumps.
const STATES_TABLE: TableDefinition<'_, &str, &[u8]> = TableDefinition::new("window_states");

/// One window's persisted state. Optional fields stay absent in the
/// stored payload when unknown — that way a window restored before the
/// host has reported its position doesn't get pinned to (0, 0).
///
/// Construct via `serde_json::from_value` (IPC / disk path) or
/// `WindowState::builder()` (Rust callers, embedders). All fields are
/// optional in the builder.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, bon::Builder)]
pub(crate) struct WindowState {
    /// Outer-position X in logical pixels.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub(crate) x: Option<i32>,
    /// Outer-position Y in logical pixels.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub(crate) y: Option<i32>,
    /// Inner-size width in logical pixels.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub(crate) width: Option<u32>,
    /// Inner-size height in logical pixels.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub(crate) height: Option<u32>,
    /// Whether the window was maximized.
    #[serde(default)]
    #[builder(default)]
    pub(crate) maximized: bool,
    /// Whether the window was fullscreen.
    #[serde(default)]
    #[builder(default)]
    pub(crate) fullscreen: bool,
}

/// Process-wide store. Lazy-init so the cost is paid only when the API
/// is actually exercised (custom builds without `api-window-state` skip
/// it entirely via the feature gate).
struct Store {
    states: StateMap,
    storage_path: RwLock<Option<PathBuf>>,
    /// Cached redb handle, keyed implicitly by the path stored in
    /// `storage_path`. Reset to `None` whenever the path changes
    /// (test reseeds, or callers swapping storage paths at runtime).
    db: RwLock<Option<DbHandle>>,
}

struct DbHandle {
    path: PathBuf,
    db: Database,
}

impl Store {
    fn new() -> Self {
        Self {
            states: StateMap::new(),
            storage_path: RwLock::new(None),
            db: RwLock::new(None),
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
pub(crate) fn set_storage_path(path: PathBuf) {
    *store().storage_path.write() = Some(path);
    // Drop any cached DB handle — the next op will reopen against the
    // new path. Keeps the OnceLock-style accessor honest about which
    // file backs the live state.
    *store().db.write() = None;
}

/// Currently saved snapshot for `label`, if any.
pub(crate) fn get(label: &str) -> Option<WindowState> {
    store().states.get(label).map(|entry| entry.clone())
}

/// Drop one entry, or every entry when `label` is `None`.
pub(crate) fn clear(label: Option<&str>) -> std::io::Result<()> {
    match label {
        Some(l) => {
            store().states.remove(l);
        },
        None => store().states.clear(),
    }
    Ok(())
}

/// Apply a snapshot to the in-memory map. Does **not** flush to disk —
/// callers batch with `save_to_disk`.
pub(crate) fn set(label: &str, state: WindowState) {
    store().states.insert(label.to_string(), state);
}

/// Read the persisted DB at `path` and replace the in-memory map.
/// Corrupted databases fall through to an empty map (the TS client's
/// next snapshot will reseed). Sets the configured storage path as
/// a side-effect so subsequent saves are unambiguous.
pub(crate) fn load_from_disk(path: PathBuf) -> std::io::Result<()> {
    let path = authorize_storage_path(&path)?;
    set_storage_path(path.clone());

    let map = read_states_or_empty(&path);
    store().states.clear();
    for (label, state) in map {
        store().states.insert(label, state);
    }
    Ok(())
}

/// Persist the in-memory map to redb. Best-effort for callers — errors
/// surface when the DB can't be opened or a write transaction fails.
pub(crate) fn save_to_disk() -> std::io::Result<()> {
    let path = store().storage_path.read().clone().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "no storage path configured — call setStoragePath / loadFromDisk first",
        )
    })?;
    let snapshot: HashMap<String, WindowState> = store()
        .states
        .iter()
        .map(|entry| (entry.key().clone(), entry.value().clone()))
        .collect();
    write_states(&path, &snapshot)
}

/// Dispatch marker — window-state store lives in `STORE`.
#[derive(Debug, Default)]
pub(crate) struct WindowStateApi;

/// Row of the host API table (`api::HOST_APIS`).
pub(crate) const API: HostApi = HostApi::new("windowState", |method, args| dispatch(method, &args));

#[tynd::handler(api = "windowState")]
impl WindowStateApi {
    /// Return the persisted snapshot for `label`, or `null` when no
    /// entry exists.
    pub fn get(label: String) -> Value {
        get(&label).map_or(Value::Null, |s| {
            serde_json::to_value(s).unwrap_or(Value::Null)
        })
    }

    /// Return every persisted snapshot, keyed by window label.
    pub fn get_all() -> Value {
        let mut out = serde_json::Map::with_capacity(store().states.len());
        for entry in &store().states {
            out.insert(
                entry.key().clone(),
                serde_json::to_value(entry.value()).unwrap_or(Value::Null),
            );
        }
        Value::Object(out)
    }

    /// Replace the in-memory snapshot for `label`. Does not flush
    /// to disk — call `saveToDisk` to materialise.
    pub fn set(label: String, state: WindowState) {
        set(&label, state);
    }

    /// Drop the snapshot for `label`. When `label` is omitted,
    /// every entry is wiped.
    pub fn clear(label: Option<String>) -> Result<(), String> {
        clear(label.as_deref()).map_err(|e| IpcError::new("windowState", "clear", e).into_string())
    }

    /// Configure the on-disk path used by `loadFromDisk` / `saveToDisk`.
    /// The redb file lives next to `path` (same directory, `.redb` extension).
    pub fn set_storage_path(path: String) -> Result<(), String> {
        let path = authorize_storage_path(Path::new(&path))
            .map_err(|e| IpcError::new("windowState", "setStoragePath", e).into_string())?;
        set_storage_path(path);
        Ok(())
    }

    /// Load the persisted state from `path` (or the
    /// previously-configured path when omitted). Corrupted databases
    /// are silently treated as an empty map — a corrupted file should
    /// never crash the host.
    pub fn load_from_disk(path: Option<String>) -> Result<(), String> {
        let path = if let Some(p) = path {
            PathBuf::from(p)
        } else {
            store().storage_path.read().clone().ok_or_else(|| {
                IpcError::new(
                    "windowState",
                    "loadFromDisk",
                    "no path provided and none configured",
                )
                .into_string()
            })?
        };
        load_from_disk(path)
            .map_err(|e| IpcError::new("windowState", "loadFromDisk", e).into_string())
    }

    /// Atomically write the in-memory snapshot to the configured
    /// storage path. Errors when no path has been set yet.
    pub fn save_to_disk() -> Result<(), String> {
        save_to_disk().map_err(|e| IpcError::new("windowState", "saveToDisk", e).into_string())
    }
}

/// Resolve the redb file path next to `path` — same directory, same
/// stem, `.redb` extension. Returns `None` when `path` doesn't have a
/// usable parent (rare; only on filesystem-root paths).
fn redb_path_for(path: &Path) -> PathBuf {
    let mut p = path.to_path_buf();
    p.set_extension("redb");
    p
}

/// Run `f` against the redb database for `path`. The handle is cached
/// in `STORE.db` so back-to-back saves don't re-acquire the file lock
/// (Windows in particular doesn't tolerate the redb-vs-redb lock
/// contention well — the cached handle holds the exclusive lock for
/// the lifetime of the storage-path session). Cache is invalidated
/// whenever `set_storage_path` switches paths.
fn with_db<R>(path: &Path, f: impl FnOnce(&Database) -> std::io::Result<R>) -> std::io::Result<R> {
    // Redb opens an ambient path and caches the handle. Re-authorize before
    // consulting that cache so a policy tightening takes effect immediately.
    let db_path = authorize_db_path(path)?;

    {
        let guard = store().db.read();
        if let Some(handle) = guard.as_ref() {
            if handle.path == db_path {
                return f(&handle.db);
            }
        }
    }

    if let Some(parent) = db_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut guard = store().db.write();
    if let Some(handle) = guard.as_ref() {
        if handle.path == db_path {
            return f(&handle.db);
        }
    }
    let db =
        Database::create(&db_path).map_err(|e| std::io::Error::other(format!("redb open: {e}")))?;
    let result = f(&db);
    *guard = Some(DbHandle { path: db_path, db });
    result
}

fn invalid_path(error: impl Into<String>) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::PermissionDenied, error.into())
}

fn authorize_path(path: &Path) -> std::io::Result<PathBuf> {
    let path = path
        .to_str()
        .ok_or_else(|| invalid_path("window-state path is not valid UTF-8"))?;
    crate::security::policy::authorize_fs_path(path).map_err(invalid_path)
}

/// Authorize both the caller-facing path and the effective `.redb`
/// sibling. The latter is the path Redb actually opens.
fn authorize_storage_path(path: &Path) -> std::io::Result<PathBuf> {
    let path = authorize_path(path)?;
    authorize_db_path(&path)?;
    Ok(path)
}

fn authorize_db_path(path: &Path) -> std::io::Result<PathBuf> {
    authorize_path(&redb_path_for(path))
}

/// Read the redb DB next to `path` and decode every row into the
/// label → state map. Returns an empty map on any failure (DB missing,
/// IO error, decode error) — corrupted state should never crash the
/// host.
fn read_states_or_empty(path: &Path) -> HashMap<String, WindowState> {
    if !redb_path_for(path).exists() {
        return HashMap::new();
    }
    with_db(path, |db| {
        let mut out = HashMap::new();
        let Ok(txn) = db.begin_read() else {
            return Ok(out);
        };
        // Any open_table failure (table-missing on first launch, schema
        // mismatch on a corrupted file) collapses to "empty map" — a
        // corrupted DB must never crash the host.
        let Ok(table) = txn.open_table(STATES_TABLE) else {
            return Ok(out);
        };
        let Ok(iter) = table.iter() else {
            return Ok(out);
        };
        for entry in iter.flatten() {
            let (k, v) = entry;
            let label = k.value().to_string();
            let bytes = v.value();
            if let Ok(state) = serde_json::from_slice::<WindowState>(bytes) {
                out.insert(label, state);
            }
        }
        Ok(out)
    })
    .unwrap_or_default()
}

/// Write every entry in `states` into the redb DB next to `path` under
/// a single commit. Replace-semantics: rows present in redb but absent
/// from `states` are dropped — every save mirrors the in-memory snapshot.
fn write_states(path: &Path, states: &HashMap<String, WindowState>) -> std::io::Result<()> {
    with_db(path, |db| {
        let txn = db
            .begin_write()
            .map_err(|e| std::io::Error::other(format!("begin_write: {e}")))?;
        {
            let mut table = txn
                .open_table(STATES_TABLE)
                .map_err(|e| std::io::Error::other(format!("open_table: {e}")))?;

            let existing: Vec<String> = table
                .iter()
                .map_err(|e| std::io::Error::other(format!("iter: {e}")))?
                .flatten()
                .map(|(k, _)| k.value().to_string())
                .collect();
            for label in existing {
                if !states.contains_key(&label) {
                    let _ = table.remove(label.as_str());
                }
            }

            for (label, state) in states {
                let bytes = serde_json::to_vec(state)
                    .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
                table
                    .insert(label.as_str(), bytes.as_slice())
                    .map_err(|e| std::io::Error::other(format!("insert: {e}")))?;
            }
        }
        txn.commit()
            .map_err(|e| std::io::Error::other(format!("commit: {e}")))?;
        Ok(())
    })
}
