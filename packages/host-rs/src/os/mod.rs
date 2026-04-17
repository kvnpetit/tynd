pub mod clipboard;
pub mod dialog;
pub mod fs;
pub mod icon;
pub mod notification;
pub mod os_info;
pub mod process;
pub mod shell;
pub mod store;
pub mod window_cmd;

use serde_json::Value;

/// Dispatch a non-window OS API call from a background thread.
/// Returns `Ok(value)` or `Err(user-facing message)`.
pub fn dispatch(api: &str, method: &str, args: &Value) -> Result<Value, String> {
    match api {
        "dialog" => dialog::dispatch(method, args),
        "clipboard" => clipboard::dispatch(method, args),
        "shell" => shell::dispatch(method, args),
        "notification" => notification::dispatch(method, args),
        "process" => process::dispatch(method, args),
        "fs" => fs::dispatch(method, args),
        "store" => store::dispatch(method, args),
        "os" => os_info::dispatch(method, args),
        _ => Err(format!("Unknown OS API: '{api}'")),
    }
}
