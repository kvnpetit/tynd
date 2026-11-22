//! `window.*` debug methods — list, info, manage (move/resize/focus/min/max).
//!
//! Every operation routes through a host-installed hook because the
//! underlying `tao::window::Window` handles aren't `Send` on every platform.

use std::sync::OnceLock;

use serde_json::Value;
use tynd_debug_protocol::protocol::RpcError;

use super::CallOutcome;

/// Manage hook contract — the host installs a closure that runs on the
/// main event loop to mutate window state. Returns a JSON value with
/// `{ ok: true, ... }` or an error message.
pub(crate) type WindowHookFn =
    Box<dyn Fn(&str, &Value) -> Result<Value, String> + Send + Sync + 'static>;

static HOOK: OnceLock<WindowHookFn> = OnceLock::new();

pub(crate) fn set_hook(f: WindowHookFn) {
    let _ = HOOK.set(f);
}

pub(crate) fn query(action: &str, params: &Value) -> Result<Value, RpcError> {
    let Some(hook) = HOOK.get() else {
        return Err(RpcError::internal_error(
            "window hook not installed — host event loop not started",
        ));
    };
    hook(action, params).map_err(RpcError::invalid_params)
}

pub(crate) fn list(params: Value) -> CallOutcome {
    match query("list", &params) {
        Ok(value) => CallOutcome::Result(value),
        Err(error) => CallOutcome::Error(error),
    }
}

pub(crate) fn info(params: Value) -> CallOutcome {
    match query("info", &params) {
        Ok(value) => CallOutcome::Result(value),
        Err(error) => CallOutcome::Error(error),
    }
}

pub(crate) fn manage(params: Value) -> CallOutcome {
    let action = match params.get("action").and_then(Value::as_str) {
        Some(s) => s.to_string(),
        None => return CallOutcome::Error(RpcError::invalid_params("missing 'action'")),
    };
    let mut flattened = params;
    if let Some(args) = flattened.get("args").and_then(Value::as_object).cloned() {
        let Some(root) = flattened.as_object_mut() else {
            return CallOutcome::Error(RpcError::invalid_params("params must be an object"));
        };
        for (key, value) in args {
            root.entry(key).or_insert(value);
        }
    }
    match query(&action, &flattened) {
        Ok(v) => CallOutcome::Result(v),
        Err(e) => CallOutcome::Error(e),
    }
}
