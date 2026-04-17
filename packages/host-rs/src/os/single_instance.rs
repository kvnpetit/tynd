use serde_json::{json, Value};
use single_instance::SingleInstance;
use std::sync::{Mutex, OnceLock};

// Lock must outlive the process; keep it in a static so the OS release
// only happens on real exit.
static LOCK: OnceLock<Mutex<Option<SingleInstance>>> = OnceLock::new();

pub fn dispatch(method: &str, args: &Value) -> Result<Value, String> {
    match method {
        "acquire" => acquire(args),
        "isAcquired" => Ok(Value::Bool(is_held())),
        _ => Err(format!("singleInstance.{method}: unknown method")),
    }
}

fn acquire(args: &Value) -> Result<Value, String> {
    let id = args
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| "singleInstance.acquire: missing 'id'".to_string())?;

    let cell = LOCK.get_or_init(|| Mutex::new(None));
    let mut guard = cell.lock().map_err(|e| e.to_string())?;
    if guard.is_some() {
        return Ok(json!({ "acquired": true, "already": true }));
    }
    let instance = SingleInstance::new(id).map_err(|e| format!("singleInstance.acquire: {e}"))?;
    let acquired = instance.is_single();
    if acquired {
        *guard = Some(instance);
    }
    Ok(json!({ "acquired": acquired, "already": false }))
}

fn is_held() -> bool {
    LOCK.get()
        .and_then(|c| c.lock().ok())
        .is_some_and(|g| g.is_some())
}
