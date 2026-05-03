//! JSON key/value store at `config_dir()/<ns>/store.json`. Loaded once per
//! namespace and cached. Writes are debounced — a background thread per
//! namespace coalesces a burst of `set`/`delete`/`clear` calls into a
//! single fsync+rename roughly every 100 ms. Use `flush` to force an
//! immediate write (shutdown / explicit sync from JS).

use std::{
    fs,
    io::Write as _,
    path::PathBuf,
    sync::{Arc, LazyLock},
    time::Duration,
};

use crossbeam_channel::{after, never, select, unbounded, Sender};
use dashmap::DashMap;
use serde_json::{Map, Value};

/// Debounce window: a set/delete/clear that arrives within this delay
/// of a previous one extends the timer instead of triggering a fresh
/// fsync+rename. 100 ms is below "user-perceptible" so a UI-driven
/// burst (e.g. tab close → `set` 8 keys) collapses to a single I/O.
const PERSIST_DEBOUNCE: Duration = Duration::from_millis(100);

/// Process-wide namespace → in-memory map cache. `LazyLock` (Rust
/// 1.80+) builds the inner `DashMap` on first deref so callers reach
/// the map via direct `STORES.get(...)` instead of a `stores()`
/// accessor wrapping `OnceLock::get_or_init`.
///
/// Map is wrapped in `Arc` so the persister thread can take a cheap
/// `Arc::clone` snapshot of the namespace state (one atomic inc) and
/// serialize it without holding any DashMap shard lock — concurrent
/// `set`/`get` proceed unblocked while the disk write happens.
static STORES: LazyLock<DashMap<String, Arc<Map<String, Value>>>> = LazyLock::new(DashMap::new);

/// Per-namespace persister handles. Each entry owns the channel that
/// feeds the background thread and a flush ack channel for sync waits.
static PERSISTERS: LazyLock<DashMap<String, Persister>> = LazyLock::new(DashMap::new);

#[derive(Debug)]
struct Persister {
    tx: Sender<PersistCmd>,
}

/// Commands routed to a per-namespace background thread. `Touch`
/// schedules a debounced flush; `Flush` forces an immediate write and
/// waits for completion via the embedded ack channel.
#[derive(Debug)]
enum PersistCmd {
    Touch,
    Flush(Sender<()>),
}

pub fn dispatch(method: &str, args: &Value) -> Result<Value, String> {
    let namespace = args
        .get("namespace")
        .and_then(|v| v.as_str())
        .unwrap_or("default")
        .to_string();

    match method {
        "get" => {
            let key = key_arg(args)?;
            load_if_needed(&namespace)?;
            Ok(STORES
                .get(&namespace)
                .and_then(|m| m.get(key).cloned())
                .unwrap_or(Value::Null))
        },
        "set" => {
            let key = key_arg(args)?;
            let value = args.get("value").cloned().unwrap_or(Value::Null);
            load_if_needed(&namespace)?;
            mutate(&namespace, |m| {
                m.insert(key.to_string(), value);
            });
            schedule_persist(&namespace)?;
            Ok(Value::Null)
        },
        "delete" => {
            let key = key_arg(args)?;
            load_if_needed(&namespace)?;
            mutate(&namespace, |m| {
                m.remove(key);
            });
            schedule_persist(&namespace)?;
            Ok(Value::Null)
        },
        "clear" => {
            // Validate before mutating in-memory state — otherwise an
            // invalid namespace would clear the in-memory map and then fail
            // to persist, leaving on-disk + in-memory desynchronised.
            validate_namespace(&namespace)?;
            STORES.insert(namespace.clone(), Arc::new(Map::new()));
            schedule_persist(&namespace)?;
            Ok(Value::Null)
        },
        "keys" => {
            load_if_needed(&namespace)?;
            let keys: Vec<Value> = STORES
                .get(&namespace)
                .map(|m| m.keys().map(|k| Value::String(k.clone())).collect())
                .unwrap_or_default();
            Ok(Value::Array(keys))
        },
        "has" => {
            let key = key_arg(args)?;
            load_if_needed(&namespace)?;
            let present = STORES
                .get(&namespace)
                .is_some_and(|m| m.contains_key(key));
            Ok(Value::Bool(present))
        },
        "values" => {
            load_if_needed(&namespace)?;
            let values: Vec<Value> = STORES
                .get(&namespace)
                .map(|m| m.values().cloned().collect())
                .unwrap_or_default();
            Ok(Value::Array(values))
        },
        "entries" => {
            load_if_needed(&namespace)?;
            // Pairs serialize as `[key, value]` tuples so the JS side can
            // destructure them straight into `Object.fromEntries` without
            // extra remapping.
            let entries: Vec<Value> = STORES
                .get(&namespace)
                .map(|m| {
                    m.iter()
                        .map(|(k, v)| Value::Array(vec![Value::String(k.clone()), v.clone()]))
                        .collect()
                })
                .unwrap_or_default();
            Ok(Value::Array(entries))
        },
        "length" => {
            load_if_needed(&namespace)?;
            let n = STORES.get(&namespace).map_or(0, |m| m.len());
            Ok(Value::Number(n.into()))
        },
        "reload" => {
            // Force any pending debounced write through before we drop
            // the in-memory copy — otherwise a `set` followed immediately
            // by `reload` would silently roll the value back to whatever
            // is on disk.
            flush_namespace(&namespace);
            STORES.remove(&namespace);
            load_if_needed(&namespace)?;
            Ok(Value::Null)
        },
        "flush" => {
            // Explicit sync — JS callers (and the host shutdown path)
            // use this to force any debounced writes through before
            // returning. No-op for namespaces with no pending state.
            flush_namespace(&namespace);
            Ok(Value::Null)
        },
        _ => Err(format!("store.{method}: unknown method")),
    }
}

/// Apply a mutation to the namespace map under copy-on-write semantics:
/// clone the current `Arc<Map>` (cheap inc), mutate the clone in place,
/// publish via `DashMap::insert`. Concurrent readers keep their old
/// snapshot until they drop it; the next `get` lands on the new one.
fn mutate<F: FnOnce(&mut Map<String, Value>)>(namespace: &str, f: F) {
    let mut entry = STORES
        .entry(namespace.to_string())
        .or_insert_with(|| Arc::new(Map::new()));
    // `Arc::make_mut` clones the inner map iff there are other strong
    // references (a persister-held snapshot, in our case). Free path
    // is mutate-in-place when no one else is reading.
    let m = Arc::make_mut(entry.value_mut());
    f(m);
}

/// Force an immediate write for `namespace`, blocking until the
/// background thread acks. Cheap when the persister has nothing
/// pending (the thread serializes the current snapshot once, fsyncs,
/// renames, and signals).
fn flush_namespace(namespace: &str) {
    let Some(p) = PERSISTERS.get(namespace) else {
        return;
    };
    let (ack_tx, ack_rx) = unbounded::<()>();
    if p.tx.send(PersistCmd::Flush(ack_tx)).is_err() {
        return;
    }
    // 5 s upper bound — disk I/O on a sane filesystem completes in
    // well under that; if it doesn't we'd rather return than wedge
    // the IPC dispatch thread.
    let _ = ack_rx.recv_timeout(Duration::from_secs(5));
}

fn key_arg(args: &Value) -> Result<&str, String> {
    args.get("key")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "store: missing 'key' argument".to_string())
}

fn validate_namespace(namespace: &str) -> Result<(), String> {
    // Namespaces become directory names under `config_dir/`. Reject anything
    // that would let a caller escape the parent directory or write to a
    // sibling app's data: empty strings, leading dots, slashes, and any
    // component outside [A-Za-z0-9_-].
    if namespace.is_empty() {
        return Err("store: namespace cannot be empty".into());
    }
    if namespace.starts_with('.') {
        return Err(format!(
            "store: namespace cannot start with '.' ('{namespace}')"
        ));
    }
    for ch in namespace.chars() {
        let ok = ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' || ch == '.';
        if !ok {
            return Err(format!(
                "store: invalid character '{ch}' in namespace '{namespace}' (allowed: A-Z, a-z, \
                 0-9, '_', '-', '.')"
            ));
        }
    }
    if namespace.contains("..") {
        return Err(format!(
            "store: namespace cannot contain '..' ('{namespace}')"
        ));
    }
    Ok(())
}

fn store_path(namespace: &str) -> Result<PathBuf, String> {
    validate_namespace(namespace)?;
    let base = dirs::config_dir()
        .or_else(dirs::data_dir)
        .ok_or_else(|| "store: no config/data directory available".to_string())?;
    Ok(base.join(namespace).join("store.json"))
}

fn load_if_needed(namespace: &str) -> Result<(), String> {
    if STORES.contains_key(namespace) {
        return Ok(());
    }
    let path = store_path(namespace)?;
    let map = if path.exists() {
        let raw = fs::read_to_string(&path)
            .map_err(|e| format!("store: read {}: {e}", path.display()))?;
        serde_json::from_str::<Map<String, Value>>(&raw).unwrap_or_default()
    } else {
        Map::new()
    };
    // `entry().or_insert` is the atomic check-and-insert — without it, two
    // concurrent threads can both miss `contains_key`, both read the file,
    // and the second `insert` would silently clobber the first (losing any
    // `set` that happened in between).
    STORES
        .entry(namespace.to_string())
        .or_insert(Arc::new(map));
    Ok(())
}

/// Notify the per-namespace persister that the in-memory map changed.
/// Spawns the background thread on first reach. Cheap unbounded send —
/// the persister coalesces a burst of `Touch`es into one disk write
/// after `PERSIST_DEBOUNCE`.
fn schedule_persist(namespace: &str) -> Result<(), String> {
    // Validate the path once up front so we don't spawn a thread for a
    // namespace that would fail at write time anyway.
    let _ = store_path(namespace)?;
    let p = PERSISTERS
        .entry(namespace.to_string())
        .or_insert_with(|| spawn_persister(namespace.to_string()));
    let _ = p.tx.send(PersistCmd::Touch);
    Ok(())
}

/// Spawn the per-namespace persister thread. Holds an unbounded
/// channel of `PersistCmd`; the loop coalesces `Touch` bursts via a
/// `crossbeam_channel::after` timer and services `Flush(ack)` requests
/// synchronously (drains the timer, writes, then sends the ack so the
/// caller can know the write hit disk).
fn spawn_persister(namespace: String) -> Persister {
    let (tx, rx) = unbounded::<PersistCmd>();
    std::thread::Builder::new()
        .name(format!("tynd-store-{namespace}"))
        .spawn(move || {
            let mut pending = false;
            loop {
                // No pending write → block on the inbound channel.
                // Pending → race the inbound channel against the
                // debounce timer; whichever fires first wins.
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
                            // Synchronous flush — write now regardless of timer state.
                            if pending {
                                let _ = persist_now(&namespace);
                                pending = false;
                            }
                            let _ = ack.send(());
                        }
                        Err(_) => {
                            // Sender dropped (process shutdown). Best-effort
                            // final flush so we don't lose pending writes.
                            if pending {
                                let _ = persist_now(&namespace);
                            }
                            return;
                        }
                    },
                    recv(timer) -> _ => {
                        if pending {
                            let _ = persist_now(&namespace);
                            pending = false;
                        }
                    }
                }
            }
        })
        .expect("store: spawn persister thread");
    Persister { tx }
}

/// Serialize the current namespace snapshot and atomically rename it
/// over `store.json`. Called from the persister thread; never blocks
/// readers (`Arc::clone` is one atomic inc, the serializer iterates
/// the snapshot independently).
fn persist_now(namespace: &str) -> Result<(), String> {
    let path = store_path(namespace)?;
    let parent = path
        .parent()
        .ok_or_else(|| format!("store: path {} has no parent dir", path.display()))?;
    fs::create_dir_all(parent).map_err(|e| format!("store: mkdir {}: {e}", parent.display()))?;
    // Cheap snapshot — `Arc::clone` is one atomic increment. The map
    // we serialize is the exact one published by the latest mutator;
    // any `set` that lands after this clone will land on a new Arc and
    // schedule a fresh `Touch` for the next debounce window.
    let data: Arc<Map<String, Value>> = STORES
        .get(namespace)
        .map(|r| Arc::clone(r.value()))
        .unwrap_or_else(|| Arc::new(Map::new()));
    let serialized = serde_json::to_string(&*data).map_err(|e| e.to_string())?;

    // Crash-safe persist: write to a sibling temp file in the same
    // directory, fsync, then atomically rename over the target.
    let mut tmp = tempfile::NamedTempFile::new_in(parent)
        .map_err(|e| format!("store: tempfile in {}: {e}", parent.display()))?;
    tmp.write_all(serialized.as_bytes())
        .map_err(|e| format!("store: write {}: {e}", path.display()))?;
    tmp.as_file_mut()
        .sync_all()
        .map_err(|e| format!("store: fsync {}: {e}", path.display()))?;
    tmp.persist(&path)
        .map_err(|e| format!("store: persist -> {}: {e}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    // Each test owns a unique namespace so they never collide with each
    // other or with a real user's stores on disk.
    fn ns(tag: &str) -> String {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        format!("tynd-test-{tag}-{ts}")
    }

    #[test]
    fn set_then_get_returns_value() {
        let n = ns("sg");
        dispatch(
            "set",
            &json!({ "namespace": &n, "key": "theme", "value": "dark" }),
        )
        .unwrap();
        let v = dispatch("get", &json!({ "namespace": &n, "key": "theme" })).unwrap();
        assert_eq!(v.as_str().unwrap(), "dark");
        let _ = fs::remove_dir_all(store_path(&n).unwrap().parent().unwrap());
    }

    #[test]
    fn missing_key_returns_null() {
        let n = ns("miss");
        let v = dispatch("get", &json!({ "namespace": &n, "key": "nope" })).unwrap();
        assert!(v.is_null());
    }

    #[test]
    fn delete_removes_key() {
        let n = ns("del");
        dispatch("set", &json!({ "namespace": &n, "key": "a", "value": 1 })).unwrap();
        dispatch("delete", &json!({ "namespace": &n, "key": "a" })).unwrap();
        let v = dispatch("get", &json!({ "namespace": &n, "key": "a" })).unwrap();
        assert!(v.is_null());
        let _ = fs::remove_dir_all(store_path(&n).unwrap().parent().unwrap());
    }

    #[test]
    fn keys_lists_every_set_key() {
        let n = ns("keys");
        dispatch("set", &json!({ "namespace": &n, "key": "a", "value": 1 })).unwrap();
        dispatch("set", &json!({ "namespace": &n, "key": "b", "value": 2 })).unwrap();
        let v = dispatch("keys", &json!({ "namespace": &n })).unwrap();
        let arr = v.as_array().unwrap();
        assert_eq!(arr.len(), 2);
        let _ = fs::remove_dir_all(store_path(&n).unwrap().parent().unwrap());
    }

    #[test]
    fn persistence_survives_namespace_reload() {
        let n = ns("persist");
        dispatch("set", &json!({ "namespace": &n, "key": "k", "value": "v" })).unwrap();
        // Force the debounced write through before evicting the
        // in-memory copy — otherwise the next `get` would hit disk
        // before the persister thread had a chance to fire.
        dispatch("flush", &json!({ "namespace": &n })).unwrap();
        // Evict the in-memory copy so the next read has to hit disk.
        STORES.remove(&n);
        let v = dispatch("get", &json!({ "namespace": &n, "key": "k" })).unwrap();
        assert_eq!(v.as_str().unwrap(), "v");
        let _ = fs::remove_dir_all(store_path(&n).unwrap().parent().unwrap());
    }

    #[test]
    fn has_reflects_set_and_delete() {
        let n = ns("has");
        assert!(!dispatch("has", &json!({ "namespace": &n, "key": "x" }))
            .unwrap()
            .as_bool()
            .unwrap());
        dispatch("set", &json!({ "namespace": &n, "key": "x", "value": 1 })).unwrap();
        assert!(dispatch("has", &json!({ "namespace": &n, "key": "x" }))
            .unwrap()
            .as_bool()
            .unwrap());
        dispatch("delete", &json!({ "namespace": &n, "key": "x" })).unwrap();
        assert!(!dispatch("has", &json!({ "namespace": &n, "key": "x" }))
            .unwrap()
            .as_bool()
            .unwrap());
        let _ = fs::remove_dir_all(store_path(&n).unwrap().parent().unwrap());
    }

    #[test]
    fn values_and_entries_and_length() {
        let n = ns("vel");
        dispatch("set", &json!({ "namespace": &n, "key": "a", "value": 1 })).unwrap();
        dispatch(
            "set",
            &json!({ "namespace": &n, "key": "b", "value": "two" }),
        )
        .unwrap();

        let len = dispatch("length", &json!({ "namespace": &n }))
            .unwrap()
            .as_u64()
            .unwrap();
        assert_eq!(len, 2);

        let values = dispatch("values", &json!({ "namespace": &n })).unwrap();
        let arr = values.as_array().unwrap();
        assert_eq!(arr.len(), 2);

        let entries = dispatch("entries", &json!({ "namespace": &n })).unwrap();
        let entries_arr = entries.as_array().unwrap();
        assert_eq!(entries_arr.len(), 2);
        for pair in entries_arr {
            let p = pair.as_array().unwrap();
            assert_eq!(p.len(), 2);
            assert!(p[0].is_string());
        }
        let _ = fs::remove_dir_all(store_path(&n).unwrap().parent().unwrap());
    }

    #[test]
    fn reload_picks_up_external_writes() {
        let n = ns("reload");
        dispatch(
            "set",
            &json!({ "namespace": &n, "key": "k", "value": "v1" }),
        )
        .unwrap();
        // Make sure v1 actually hits disk before we overwrite the file
        // out-of-band — without this, the debounce timer might still
        // be pending and our external write would race with the
        // persister thread.
        dispatch("flush", &json!({ "namespace": &n })).unwrap();

        // Simulate an out-of-process write: rewrite the JSON file directly.
        let path = store_path(&n).unwrap();
        fs::write(&path, r#"{"k":"v2","extra":42}"#).unwrap();

        // Without reload the cached value is stale.
        let cached = dispatch("get", &json!({ "namespace": &n, "key": "k" }))
            .unwrap()
            .as_str()
            .unwrap()
            .to_string();
        assert_eq!(cached, "v1");

        dispatch("reload", &json!({ "namespace": &n })).unwrap();
        let fresh = dispatch("get", &json!({ "namespace": &n, "key": "k" }))
            .unwrap()
            .as_str()
            .unwrap()
            .to_string();
        assert_eq!(fresh, "v2");
        assert_eq!(
            dispatch("length", &json!({ "namespace": &n }))
                .unwrap()
                .as_u64()
                .unwrap(),
            2
        );
        let _ = fs::remove_dir_all(store_path(&n).unwrap().parent().unwrap());
    }
}
