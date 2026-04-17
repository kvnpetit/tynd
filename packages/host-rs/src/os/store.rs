//! JSON key/value store at `config_dir()/<ns>/store.json`. Loaded once per
//! namespace and cached; writes flush synchronously.

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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
        // Evict the in-memory copy so the next read has to hit disk.
        stores().lock().unwrap().remove(&n);
        let v = dispatch("get", &json!({ "namespace": &n, "key": "k" })).unwrap();
        assert_eq!(v.as_str().unwrap(), "v");
        let _ = fs::remove_dir_all(store_path(&n).unwrap().parent().unwrap());
    }
}
