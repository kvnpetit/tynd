//! Registry of sidecar binaries extracted from the TYNDPKG `sidecar/*` section
//! at startup. Populated by the full/lite embed loaders.

use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

static SIDECARS: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();

fn registry() -> &'static Mutex<HashMap<String, String>> {
    SIDECARS.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn register(name: &str, absolute_path: &str) {
    if let Ok(mut m) = registry().lock() {
        m.insert(name.to_string(), absolute_path.to_string());
    }
}

pub fn dispatch(method: &str, args: &Value) -> Result<Value, String> {
    match method {
        "path" => {
            let name = args
                .get("name")
                .and_then(Value::as_str)
                .ok_or_else(|| "sidecar.path: missing 'name'".to_string())?;
            let map = registry().lock().map_err(|e| e.to_string())?;
            map.get(name).cloned().map_or_else(
                || Err(format!("sidecar '{name}' is not registered")),
                |p| Ok(Value::String(p)),
            )
        },
        "list" => {
            let map = registry().lock().map_err(|e| e.to_string())?;
            let items: Vec<Value> = map
                .iter()
                .map(|(name, path)| json!({ "name": name, "path": path }))
                .collect();
            Ok(Value::Array(items))
        },
        _ => Err(format!("sidecar.{method}: unknown method")),
    }
}
