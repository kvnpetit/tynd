pub mod app;
pub mod autolaunch;
pub mod call_pool;
pub mod clipboard;
pub mod compute;
pub mod dialog;
pub mod events;
pub mod fs;
pub mod http;
pub mod icon;
pub mod keyring;
pub mod log;
pub mod notification;
pub mod os_info;
pub mod power;
pub mod process;
pub mod security;
pub mod shell;
pub mod shortcuts;
pub mod sidecar;
pub mod single_instance;
pub mod sql;
pub mod store;
pub mod terminal;
pub mod updater;
pub mod websocket;
pub mod window_cmd;

#[cfg(feature = "embedded-js")]
pub mod workers;

use serde_json::Value;

/// Dispatch a non-window OS API call from a background thread.
/// Returns `Ok(value)` or `Err(user-facing message)`.
pub fn dispatch(api: &str, method: &str, args: &Value) -> Result<Value, String> {
    match api {
        "app" => app::dispatch(method, args),
        "autolaunch" => autolaunch::dispatch(method, args),
        "dialog" => dialog::dispatch(method, args),
        "clipboard" => clipboard::dispatch(method, args),
        "shell" => shell::dispatch(method, args),
        "notification" => notification::dispatch(method, args),
        "process" => process::dispatch(method, args),
        "fs" => fs::dispatch(method, args),
        "store" => store::dispatch(method, args),
        "os" => os_info::dispatch(method, args),
        "http" => http::dispatch(method, args),
        "sidecar" => sidecar::dispatch(method, args),
        "terminal" => terminal::dispatch(method, args),
        "compute" => compute::dispatch(method, args),
        "shortcuts" => shortcuts::dispatch(method, args),
        "singleInstance" => single_instance::dispatch(method, args),
        "websocket" => websocket::dispatch(method, args),
        "sql" => sql::dispatch(method, args),
        "updater" => updater::dispatch(method, args),
        "keyring" => keyring::dispatch(method, args),
        "log" => log::dispatch(method, args),
        "power" => power::dispatch(method, args),
        "security" => security::dispatch(method, args),
        #[cfg(feature = "embedded-js")]
        "workers" => workers::dispatch(method, args),
        #[cfg(not(feature = "embedded-js"))]
        "workers" => {
            Err("workers API is lite-only; in full runtime use Bun.Worker natively".to_string())
        },
        _ => Err(format!("Unknown OS API: '{api}'")),
    }
}
