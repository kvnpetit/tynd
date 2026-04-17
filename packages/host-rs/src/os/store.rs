//! Persistent key/value store scoped per app.
//!
//! Backed by a single JSON file under `dirs::config_dir()/<namespace>/store.json`
//! (or `dirs::data_dir()` fallback). Loaded lazily once per namespace and cached
//! in memory for the process lifetime. Writes are sync-flushed after each mutation.

use serde_json::{Map, Value};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

static STORES: OnceLock<Mutex<HashMap<String, Map<String, Value>>>> = OnceLock::new();

fn stores() -> &'static Mutex<HashMap<String, Map<String, Value>>> {
    STORES.get_or_init(|| Mutex::new(HashMap::new()))
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
            let map = stores().lock().map_err(|e| e.to_string())?;
            Ok(map
                .get(&namespace)
                .and_then(|m| m.get(key))
                .cloned()
                .unwrap_or(Value::Null))
        },
        "set" => {
            let key = key_arg(args)?;
            let value = args.get("value").cloned().unwrap_or(Value::Null);
            load_if_needed(&namespace)?;
            {
                let mut all = stores().lock().map_err(|e| e.to_string())?;
                let m = all.entry(namespace.clone()).or_insert_with(Map::new);
                m.insert(key.to_string(), value);
            }
            persist(&namespace)?;
            Ok(Value::Null)
        },
        "delete" => {
            let key = key_arg(args)?;
            load_if_needed(&namespace)?;
            {
                let mut all = stores().lock().map_err(|e| e.to_string())?;
                if let Some(m) = all.get_mut(&namespace) {
                    m.remove(key);
                }
            }
            persist(&namespace)?;
            Ok(Value::Null)
        },
        "clear" => {
            {
                let mut all = stores().lock().map_err(|e| e.to_string())?;
                all.insert(namespace.clone(), Map::new());
            }
            persist(&namespace)?;
            Ok(Value::Null)
        },
        "keys" => {
            load_if_needed(&namespace)?;
            let map = stores().lock().map_err(|e| e.to_string())?;
            let keys: Vec<Value> = map
                .get(&namespace)
                .map(|m| m.keys().map(|k| Value::String(k.clone())).collect())
                .unwrap_or_default();
            Ok(Value::Array(keys))
        },
        _ => Err(format!("store.{method}: unknown method")),
    }
}

fn key_arg(args: &Value) -> Result<&str, String> {
    args.get("key")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "store: missing 'key' argument".to_string())
}

fn store_path(namespace: &str) -> Result<PathBuf, String> {
    let base = dirs::config_dir()
        .or_else(dirs::data_dir)
        .ok_or_else(|| "store: no config/data directory available".to_string())?;
    Ok(base.join(namespace).join("store.json"))
}

fn load_if_needed(namespace: &str) -> Result<(), String> {
    {
        let all = stores().lock().map_err(|e| e.to_string())?;
        if all.contains_key(namespace) {
            return Ok(());
        }
    }
    let path = store_path(namespace)?;
    let map = if path.exists() {
        let raw = fs::read_to_string(&path)
            .map_err(|e| format!("store: read {}: {e}", path.display()))?;
        serde_json::from_str::<Map<String, Value>>(&raw).unwrap_or_default()
    } else {
        Map::new()
    };
    let mut all = stores().lock().map_err(|e| e.to_string())?;
    all.insert(namespace.to_string(), map);
    Ok(())
}

fn persist(namespace: &str) -> Result<(), String> {
    let path = store_path(namespace)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("store: mkdir {}: {e}", parent.display()))?;
    }
    let all = stores().lock().map_err(|e| e.to_string())?;
    let data = all.get(namespace).cloned().unwrap_or_default();
    let serialized = serde_json::to_string(&data).map_err(|e| e.to_string())?;
    fs::write(&path, serialized).map_err(|e| format!("store: write {}: {e}", path.display()))?;
    Ok(())
}
