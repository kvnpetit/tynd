//! JSON key/value store at `config_dir()/<app_id>/<ns>/store.json`, where
//! `<app_id>` is the executable's file stem. Loaded once per
//! namespace and cached. Writes are debounced — a background thread per
//! namespace coalesces a burst of `set`/`delete`/`clear` calls into a
//! single fsync+rename roughly every 100 ms. Use `flush` to force an
//! immediate write (shutdown / explicit sync from JS).
//!
//! # Inter-process safety
//!
//! No file lock. Two Tynd processes writing the same `<ns>/store.json`
//! land in last-writer-wins on the atomic rename — the loser's debounced
//! mutations are lost. Each Tynd app must ship a unique executable name so
//! the `config_dir()/<app_id>/<ns>` paths don't collide across apps.
//! In-process the `DashMap` shard locks + the `entry().or_insert()` path
//! in `load_if_needed` keep concurrent reader/writer threads coherent.

use std::{
    fs,
    io::Write as _,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, LazyLock,
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

use crossbeam_channel::{after, bounded, never, select, Sender, TrySendError};
use dashmap::DashMap;
use parking_lot::{Mutex, MutexGuard};
use serde_json::{Map, Value};

use crate::api::HostApi;
use crate::ipc::error::IpcError;
use crate::tynd;

/// Build the `store.<method>: <msg>` IPC error string.
#[cold]
fn store_err(method: &str, msg: impl std::fmt::Display) -> String {
    IpcError::new("store", method, msg).into_string()
}

/// Debounce window: a set/delete/clear that arrives within this delay
/// of a previous one extends the timer instead of triggering a fresh
/// fsync+rename. 100 ms is below "user-perceptible" so a UI-driven
/// burst (e.g. tab close → `set` 8 keys) collapses to a single I/O.
const PERSIST_DEBOUNCE: Duration = Duration::from_millis(100);
/// One queued command is enough: `Touch` is idempotent and a queued
/// `Flush` always writes the latest in-memory snapshot.
const PERSIST_QUEUE_LIMIT: usize = 1;
/// One deadline covers every namespace flush and worker join. A wedged disk
/// must not multiply cleanup latency by the number of stores an app opened.
const PERSIST_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);

/// namespace → in-memory map cache. `Arc<Map>` so the persister thread can
/// snapshot via Arc::clone and serialize without holding a shard lock.
static STORES: LazyLock<DashMap<String, Arc<Map<String, Value>>>> = LazyLock::new(DashMap::new);

#[derive(Debug, Default)]
struct PersistLifecycle {
    shutting_down: bool,
}

#[derive(Debug, Default)]
struct PersistRuntime {
    lifecycle: Mutex<PersistLifecycle>,
    persisters: DashMap<String, Persister>,
}

impl PersistRuntime {
    fn ensure_open(&self) -> Result<MutexGuard<'_, PersistLifecycle>, String> {
        let lifecycle = self.lifecycle.lock();
        if lifecycle.shutting_down {
            return Err(store_err("*", "host is shutting down"));
        }
        Ok(lifecycle)
    }

    fn shutdown(&self) {
        let persisters = {
            let mut lifecycle = self.lifecycle.lock();
            if lifecycle.shutting_down {
                return;
            }
            lifecycle.shutting_down = true;
            let names: Vec<String> = self
                .persisters
                .iter()
                .map(|entry| entry.key().clone())
                .collect();
            names
                .into_iter()
                .filter_map(|name| {
                    self.persisters
                        .remove(&name)
                        .map(|(_, persister)| persister)
                })
                .collect()
        };
        stop_persisters(persisters);
    }
}

/// Process-wide persister ownership and write admission.
static PERSIST_RUNTIME: LazyLock<PersistRuntime> = LazyLock::new(PersistRuntime::default);

#[derive(Debug)]
struct Persister {
    tx: Sender<PersistCmd>,
    dirty: Arc<AtomicBool>,
    join: JoinHandle<()>,
}

/// Commands routed to a per-namespace background thread. `Touch`
/// schedules a debounced flush; `Flush` forces an immediate write and
/// waits for completion via the embedded ack channel.
#[derive(Debug)]
enum PersistCmd {
    Touch,
    Flush(Sender<Result<(), String>>),
}

/// Dispatch marker — store state lives in `STORES` / `PERSIST_RUNTIME` below.
#[derive(Debug, Default)]
pub(crate) struct StoreApi;

/// Row of the host API table (`api::HOST_APIS`).
pub(crate) const API: HostApi = HostApi {
    shutdown: Some(shutdown),
    ..HostApi::new("store", |method, args| dispatch(method, &args))
};

#[tynd::handler(api = "store")]
impl StoreApi {
    /// Read the value associated with `key` in `namespace`. Returns
    /// `null` when the key (or the entire namespace) is absent.
    pub fn get(namespace: Option<String>, key: String) -> Result<Value, String> {
        let namespace = ns(namespace);
        load_if_needed(&namespace)?;
        Ok(STORES
            .get(&namespace)
            .and_then(|m| m.get(&key).cloned())
            .unwrap_or(Value::Null))
    }

    /// Insert or replace `value` at `key` in `namespace`. Schedules a
    /// debounced fsync.
    pub fn set(namespace: Option<String>, key: String, value: Option<Value>) -> Result<(), String> {
        let namespace = ns(namespace);
        let value = value.unwrap_or(Value::Null);
        load_if_needed(&namespace)?;
        let admission = PERSIST_RUNTIME.ensure_open()?;
        mutate(&namespace, |m| {
            m.insert(key, value);
        });
        schedule_persist(&namespace, &admission)
    }

    /// Remove `key` from `namespace`. No-op when the key is absent.
    pub fn delete(namespace: Option<String>, key: String) -> Result<(), String> {
        let namespace = ns(namespace);
        load_if_needed(&namespace)?;
        let admission = PERSIST_RUNTIME.ensure_open()?;
        mutate(&namespace, |m| {
            m.remove(&key);
        });
        schedule_persist(&namespace, &admission)
    }

    /// Drop every key in `namespace`. The next `flush` materialises
    /// the empty state to disk.
    pub fn clear(namespace: Option<String>) -> Result<(), String> {
        let namespace = ns(namespace);
        // Validate before mutating in-memory state — otherwise an
        // invalid namespace would clear the in-memory map and then fail
        // to persist, leaving on-disk + in-memory desynchronised.
        validate_namespace(&namespace)?;
        let admission = PERSIST_RUNTIME.ensure_open()?;
        STORES.insert(namespace.clone(), Arc::new(Map::new()));
        schedule_persist(&namespace, &admission)
    }

    /// List every key in `namespace`. Order is the underlying
    /// `serde_json::Map` insertion order (currently hash-table — do
    /// not rely on it).
    pub fn keys(namespace: Option<String>) -> Result<Vec<Value>, String> {
        let namespace = ns(namespace);
        load_if_needed(&namespace)?;
        Ok(STORES
            .get(&namespace)
            .map(|m| m.keys().map(|k| Value::String(k.clone())).collect())
            .unwrap_or_default())
    }

    /// True when `key` exists in `namespace`.
    pub fn has(namespace: Option<String>, key: String) -> Result<bool, String> {
        let namespace = ns(namespace);
        load_if_needed(&namespace)?;
        Ok(STORES.get(&namespace).is_some_and(|m| m.contains_key(&key)))
    }

    /// List every value in `namespace`.
    pub fn values(namespace: Option<String>) -> Result<Vec<Value>, String> {
        let namespace = ns(namespace);
        load_if_needed(&namespace)?;
        Ok(STORES
            .get(&namespace)
            .map(|m| m.values().cloned().collect())
            .unwrap_or_default())
    }

    /// List every `[key, value]` pair in `namespace`. Tuple shape so
    /// JS callers can pipe directly into `Object.fromEntries`.
    pub fn entries(namespace: Option<String>) -> Result<Vec<Value>, String> {
        let namespace = ns(namespace);
        load_if_needed(&namespace)?;
        Ok(STORES
            .get(&namespace)
            .map(|m| {
                m.iter()
                    .map(|(k, v)| Value::Array(vec![Value::String(k.clone()), v.clone()]))
                    .collect()
            })
            .unwrap_or_default())
    }

    /// Number of keys in `namespace` (0 when the namespace is
    /// uninitialised).
    pub fn length(namespace: Option<String>) -> Result<u64, String> {
        let namespace = ns(namespace);
        load_if_needed(&namespace)?;
        Ok(STORES.get(&namespace).map_or(0, |m| m.len()) as u64)
    }

    /// Drop the in-memory namespace and re-read from disk. Flushes any
    /// pending debounced write first to avoid silent rollback.
    /// Atomically replaces the in-memory map, so a racing `set()` cannot
    /// slip in between a remove and a re-load and leave stale disk state
    /// behind.
    pub fn reload(namespace: Option<String>) -> Result<(), String> {
        let namespace = ns(namespace);
        flush_namespace(&namespace)?;
        let map = read_namespace_from_disk(&namespace)?;
        STORES.insert(namespace, Arc::new(map));
        Ok(())
    }

    /// Force the debounced write through, blocking until the background
    /// thread acks. No-op when nothing pending. Rejects when the write
    /// fails or the persister does not answer within 5 s.
    pub fn flush(namespace: Option<String>) -> Result<(), String> {
        let namespace = ns(namespace);
        flush_namespace(&namespace)
    }
}

fn ns(namespace: Option<String>) -> String {
    namespace.unwrap_or_else(|| "default".into())
}

/// Copy-on-write mutation: `Arc::make_mut` clones only if a persister
/// snapshot is live; otherwise mutates in place.
fn mutate<F: FnOnce(&mut Map<String, Value>)>(namespace: &str, f: F) {
    let mut entry = STORES
        .entry(namespace.to_string())
        .or_insert_with(|| Arc::new(Map::new()));
    let m = Arc::make_mut(entry.value_mut());
    f(m);
}

/// Force-flush `namespace`, blocking until the persister acks with the
/// write result.
fn flush_namespace(namespace: &str) -> Result<(), String> {
    // Clone the sender and drop the shard read guard before blocking on
    // the ack — otherwise an unrelated `schedule_persist` whose key hashes
    // to the same DashMap shard parks for up to 5 s when the disk wedges.
    let tx = match PERSIST_RUNTIME.persisters.get(namespace) {
        Some(p) => p.tx.clone(),
        None => return Ok(()),
    };
    let (ack_tx, ack_rx) = bounded::<Result<(), String>>(1);
    let unanswered = || {
        store_err(
            "flush",
            format!("persister for namespace '{namespace}' did not answer within 5 s"),
        )
    };
    tx.send_timeout(PersistCmd::Flush(ack_tx), Duration::from_secs(5))
        .map_err(|_| unanswered())?;
    // 5 s upper bound so a wedged disk doesn't block the IPC thread.
    ack_rx
        .recv_timeout(Duration::from_secs(5))
        .map_err(|_| unanswered())?
}

/// Close write admission, flush every live namespace under one global
/// deadline, disconnect workers, and join every worker that exits in time.
/// A worker stuck in filesystem I/O is detached after the deadline so process
/// cleanup remains bounded; responsive workers flush on the explicit command
/// or when their command channel disconnects.
pub(crate) fn shutdown() {
    PERSIST_RUNTIME.shutdown();
}

fn stop_persisters(persisters: Vec<Persister>) {
    let deadline = Instant::now() + PERSIST_SHUTDOWN_TIMEOUT;

    // Signal every namespace before waiting for any one of them. A persister
    // blocked on a wedged disk must not consume the shared deadline before
    // responsive workers are told to flush.
    for persister in &persisters {
        let (acknowledgement, _receiver) = bounded::<Result<(), String>>(1);
        let _ = persister.tx.try_send(PersistCmd::Flush(acknowledgement));
    }

    let mut joins = Vec::with_capacity(persisters.len());
    for Persister { tx, dirty: _, join } in persisters {
        drop(tx);
        joins.push(join);
    }

    while !joins.is_empty() && Instant::now() < deadline {
        let mut index = 0;
        while index < joins.len() {
            if joins[index].is_finished() {
                let join = joins.swap_remove(index);
                let _ = join.join();
            } else {
                index += 1;
            }
        }
        if !joins.is_empty() {
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    if !joins.is_empty() {
        crate::tynd_warn!(
            "store shutdown deadline elapsed with {} persister worker(s) still blocked",
            joins.len()
        );
    }
}

/// Validation cache — namespaces are bounded (one per config + dynamic).
static VALIDATED_NS: LazyLock<DashMap<String, ()>> = LazyLock::new(DashMap::default);

fn validate_namespace(namespace: &str) -> Result<(), String> {
    if VALIDATED_NS.contains_key(namespace) {
        return Ok(());
    }
    validate_namespace_uncached(namespace)?;
    VALIDATED_NS.insert(namespace.to_string(), ());
    Ok(())
}

#[cold]
fn validate_namespace_uncached(namespace: &str) -> Result<(), String> {
    // Namespaces become directory names — reject anything that could escape
    // the parent or write to a sibling app's data.
    if namespace.is_empty() {
        return Err(store_err("*", "namespace cannot be empty"));
    }
    if namespace.starts_with('.') {
        return Err(store_err(
            "*",
            format!("namespace cannot start with '.' ('{namespace}')"),
        ));
    }
    for ch in namespace.chars() {
        let ok = ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' || ch == '.';
        if !ok {
            return Err(store_err(
                "*",
                format!(
                    "invalid character '{ch}' in namespace '{namespace}' (allowed: A-Z, a-z, 0-9, \
                     '_', '-', '.')"
                ),
            ));
        }
    }
    if namespace.contains("..") {
        return Err(store_err(
            "*",
            format!("namespace cannot contain '..' ('{namespace}')"),
        ));
    }
    Ok(())
}

fn store_base_dir() -> Result<PathBuf, String> {
    dirs::config_dir()
        .or_else(dirs::data_dir)
        .ok_or_else(|| store_err("*", "no config/data directory available"))
}

/// Per-app directory name: the executable's file stem, the same scoping the
/// log and updater-state files use.
fn app_dir_name() -> &'static str {
    static NAME: LazyLock<String> = LazyLock::new(|| {
        std::env::current_exe()
            .ok()
            .and_then(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
            .filter(|s| !s.is_empty() && !s.starts_with('.'))
            .unwrap_or_else(|| "tynd-app".to_string())
    });
    NAME.as_str()
}

/// `<base>/<app>/<ns>/store.json`.
fn store_path_under(base: &Path, app: &str, namespace: &str) -> PathBuf {
    base.join(app).join(namespace).join("store.json")
}

fn store_path(namespace: &str) -> Result<PathBuf, String> {
    validate_namespace(namespace)?;
    Ok(store_path_under(
        &store_base_dir()?,
        app_dir_name(),
        namespace,
    ))
}

fn load_if_needed(namespace: &str) -> Result<(), String> {
    if STORES.contains_key(namespace) {
        return Ok(());
    }
    let map = read_namespace_from_disk(namespace)?;
    // `entry().or_insert` is the atomic check-and-insert; bare `insert`
    // would let two concurrent loaders clobber each other.
    STORES.entry(namespace.to_string()).or_insert(Arc::new(map));
    Ok(())
}

/// Read + decode the on-disk store for `namespace`. Shared between the
/// lazy `load_if_needed` and the explicit `reload` (which atomic-replaces
/// the in-memory entry afterwards). Returns an empty map for a missing
/// file (first-run), errors loudly on corrupt JSON.
fn read_namespace_from_disk(namespace: &str) -> Result<Map<String, Value>, String> {
    let path = store_path(namespace)?;
    read_store_file(namespace, &path)
}

fn read_store_file(namespace: &str, path: &Path) -> Result<Map<String, Value>, String> {
    match fs::read_to_string(path) {
        Ok(raw) => match serde_json::from_str::<Map<String, Value>>(&raw) {
            Ok(m) => Ok(m),
            Err(e) => {
                tracing::error!(
                    target: "tynd::store",
                    namespace,
                    path = %path.display(),
                    error = %e,
                    "corrupt store JSON; refusing to load"
                );
                Err(store_err(
                    "*",
                    format!("corrupt store at {}: {e}", path.display()),
                ))
            },
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Map::new()),
        Err(e) => Err(store_err("*", e)),
    }
}

/// Notify the per-namespace persister that the in-memory map changed.
/// Spawns the background thread on first reach. The one-slot queue
/// coalesces bursts of `Touch` into one disk write after
/// `PERSIST_DEBOUNCE`; a full queue already contains either a `Touch` or
/// a `Flush`, both of which persist the latest in-memory snapshot.
fn schedule_persist(
    namespace: &str,
    _admission: &MutexGuard<'_, PersistLifecycle>,
) -> Result<(), String> {
    // Validate the path once up front so we don't spawn a thread for a
    // namespace that would fail at write time anyway.
    let _ = store_path(namespace)?;
    match PERSIST_RUNTIME.persisters.entry(namespace.to_string()) {
        dashmap::mapref::entry::Entry::Occupied(entry) => {
            entry.get().dirty.store(true, Ordering::Release);
            match entry.get().tx.try_send(PersistCmd::Touch) {
                Ok(()) | Err(TrySendError::Full(_)) => Ok(()),
                Err(TrySendError::Disconnected(_)) => {
                    entry.remove();
                    Err(store_err(
                        "*",
                        format!("persister for namespace '{namespace}' stopped unexpectedly"),
                    ))
                },
            }
        },
        dashmap::mapref::entry::Entry::Vacant(entry) => {
            let persister = spawn_persister(namespace.to_string())?;
            persister.dirty.store(true, Ordering::Release);
            persister.tx.send(PersistCmd::Touch).map_err(|_| {
                store_err(
                    "*",
                    format!("persister for namespace '{namespace}' stopped during startup"),
                )
            })?;
            entry.insert(persister);
            Ok(())
        },
    }
}

/// Per-namespace persister thread. Coalesces `Touch` bursts via a
/// debounce timer; `Flush(ack)` drains the timer, writes, then acks.
fn spawn_persister(namespace: String) -> Result<Persister, String> {
    let (tx, rx) = bounded::<PersistCmd>(PERSIST_QUEUE_LIMIT);
    let dirty = Arc::new(AtomicBool::new(false));
    let worker_dirty = Arc::clone(&dirty);
    let join = std::thread::Builder::new()
        .name(format!("tynd-store-{namespace}"))
        .spawn(move || {
            let mut pending = false;
            loop {
                // No pending write → block; pending → race the inbound
                // channel against the debounce timer.
                let timer = if pending {
                    after(PERSIST_DEBOUNCE)
                } else {
                    never()
                };
                select! {
                    recv(rx) -> msg => match msg {
                        Ok(PersistCmd::Touch) => {
                            pending = true;
                        }
                        Ok(PersistCmd::Flush(ack)) => {
                            // A coalesced `Touch` can observe this queued
                            // flush and skip enqueueing. The shared dirty
                            // flag preserves that mutation without making
                            // a clean reload overwrite an external write.
                            let result = persist_dirty(&namespace, "flush", &worker_dirty);
                            pending = worker_dirty.load(Ordering::Acquire);
                            let _ = ack.send(result);
                        }
                        Err(_) => {
                            let _ = persist_dirty(&namespace, "shutdown", &worker_dirty);
                            return;
                        }
                    },
                    recv(timer) -> _ => {
                        let _ = persist_dirty(&namespace, "touch", &worker_dirty);
                        pending = worker_dirty.load(Ordering::Acquire);
                    }
                }
            }
        })
        .map_err(|error| store_err("*", format!("failed to spawn persister thread: {error}")))?;
    Ok(Persister { tx, dirty, join })
}

/// Write `namespace` when dirty. Failures are reported (log + `store:error`)
/// here; the result is returned for callers that wait on it (`flush`).
fn persist_dirty(namespace: &str, phase: &str, dirty: &AtomicBool) -> Result<(), String> {
    if !dirty.swap(false, Ordering::AcqRel) {
        return Ok(());
    }
    persist_now(namespace).inspect_err(|error| {
        // Preserve retry state. A later debounce tick or explicit flush
        // retries instead of silently considering the failed write clean.
        dirty.store(true, Ordering::Release);
        report_persist_failure(namespace, phase, error);
    })
}

/// Surface a persister failure both to the structured tracing layer
/// (so headless / production builds see it in logs) AND to the frontend
/// via a `store:error` event so JS callers can fail loud instead of
/// silently observing stale on-disk state.
fn report_persist_failure(namespace: &str, phase: &str, err: &str) {
    tracing::error!(
        target: "tynd::store",
        namespace,
        phase,
        error = err,
        "store persist failed"
    );
    let path_str =
        store_path(namespace).map_or_else(|_| String::new(), |p| p.display().to_string());
    crate::ipc::events::emit(
        "store:error",
        &serde_json::json!({
            "namespace": namespace,
            "phase": phase,
            "path": path_str,
            "error": err,
        }),
    );
}

/// Serialize the snapshot and atomically rename over `store.json`. Never
/// blocks readers — `Arc::clone` is one atomic inc.
fn persist_now(namespace: &str) -> Result<(), String> {
    let path = store_path(namespace)?;
    // Cheap snapshot — any `set` that lands after this clone hits a new Arc
    // and schedules a fresh `Touch`.
    let data: Arc<Map<String, Value>> = STORES
        .get(namespace)
        .map_or_else(|| Arc::new(Map::new()), |r| Arc::clone(r.value()));
    write_store_file(&path, &data)
}

/// Crash-safe write: tempfile + fsync + atomic rename.
fn write_store_file(path: &Path, data: &Map<String, Value>) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| store_err("*", format!("path {} has no parent dir", path.display())))?;
    fs::create_dir_all(parent).map_err(|e| store_err("*", e))?;
    let serialized = serde_json::to_string(data).map_err(|e| e.to_string())?;

    let mut tmp = tempfile::NamedTempFile::new_in(parent)
        .map_err(|e| store_err("*", format!("tempfile in {}: {e}", parent.display())))?;
    tmp.write_all(serialized.as_bytes())
        .map_err(|e| store_err("*", format!("write {}: {e}", path.display())))?;
    tmp.as_file_mut()
        .sync_all()
        .map_err(|e| store_err("*", format!("fsync {}: {e}", path.display())))?;
    tmp.persist(path)
        .map_err(|e| store_err("*", format!("persist -> {}: {e}", path.display())))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn sample() -> Map<String, Value> {
        let mut m = Map::new();
        m.insert("k".into(), json!({ "a": 1 }));
        m
    }

    #[test]
    fn path_includes_app_segment() {
        let base = Path::new("cfg");
        let p = store_path_under(base, "my-app", "settings");
        assert_eq!(p, base.join("my-app").join("settings").join("store.json"));
    }

    #[test]
    fn two_apps_do_not_share_a_namespace_file() {
        let base = Path::new("cfg");
        assert_ne!(
            store_path_under(base, "app-a", "settings"),
            store_path_under(base, "app-b", "settings")
        );
    }

    #[test]
    fn namespace_validation_rejects_traversal() {
        for bad in ["", ".hidden", "a/b", "a\\b", "a..b", "x y", "../x"] {
            assert!(validate_namespace_uncached(bad).is_err(), "{bad:?}");
        }
        for ok in ["default", "my-ns_1", "v1.2"] {
            assert!(validate_namespace_uncached(ok).is_ok(), "{ok:?}");
        }
    }

    #[test]
    fn write_then_read_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = store_path_under(dir.path(), "app", "ns");
        write_store_file(&path, &sample()).unwrap();
        assert_eq!(read_store_file("ns", &path).unwrap(), sample());
    }

    #[test]
    fn missing_file_reads_as_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = store_path_under(dir.path(), "app", "ns");
        assert!(read_store_file("ns", &path).unwrap().is_empty());
    }

    #[test]
    fn corrupt_file_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = store_path_under(dir.path(), "app", "ns");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "{ not json").unwrap();
        assert!(read_store_file("ns", &path).is_err());
    }
}
