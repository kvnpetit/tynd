//! Registry of sidecar binaries extracted from the TYNDPKG `sidecar/*` section
//! at startup. Populated by the full/lite embed loaders.
//!
//! Backed by `DashMap` — reads and writes run lock-free in the common case
//! (sharded internal mutex only on contention), so a burst of concurrent
//! `sidecar.path(name)` calls don't serialise behind one global lock.

use std::sync::LazyLock;

use dashmap::DashMap;
use serde_json::{json, Value};

/// `LazyLock` (Rust 1.80+) builds the `DashMap` on first deref so
/// callers reach it via direct `SIDECARS.get(...)` instead of a
/// `registry()` accessor wrapping `OnceLock::get_or_init`.
static SIDECARS: LazyLock<DashMap<String, String>> = LazyLock::new(DashMap::new);

pub fn register(name: &str, absolute_path: &str) {
    SIDECARS.insert(name.to_string(), absolute_path.to_string());
}

pub fn dispatch(method: &str, args: &Value) -> Result<Value, String> {
    match method {
        "path" => {
            let name = args
                .get("name")
                .and_then(Value::as_str)
                .ok_or_else(|| "sidecar.path: missing 'name'".to_string())?;
            SIDECARS.get(name).map_or_else(
                || Err(format!("sidecar '{name}' is not registered")),
                |r| Ok(Value::String(r.value().clone())),
            )
        },
        "list" => {
            let items: Vec<Value> = SIDECARS
                .iter()
                .map(|r| json!({ "name": r.key(), "path": r.value() }))
                .collect();
            Ok(Value::Array(items))
        },
        _ => Err(format!("sidecar.{method}: unknown method")),
    }
}
