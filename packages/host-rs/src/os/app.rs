//! App-level APIs: name / version / exit / relaunch.
//!
//! Name + version are set by the backend on startup via `app.setInfo`.
//! The frontend then queries via `app.getName` / `app.getVersion`.
//! Exit + relaunch are uniform across runtimes.

use std::{sync::LazyLock, time::Duration};

use parking_lot::Mutex;
use serde_json::Value;

use crate::ipc_error::IpcError;

// `LazyLock` initializes the inner `Mutex<String>` to `String::new()`
// on first access — empty string is the sentinel for "the backend
// hasn't called `app.setInfo` yet" so the getters fall back to the
// process-derived default. The previous `OnceLock` shape used `None`
// for the same purpose; the empty-string check is equivalent and saves
// the per-call `Option::map` boilerplate.
static APP_NAME: LazyLock<Mutex<String>> = LazyLock::new(|| Mutex::new(String::new()));
static APP_VERSION: LazyLock<Mutex<String>> = LazyLock::new(|| Mutex::new(String::new()));
static BUNDLE_ID: LazyLock<Mutex<String>> = LazyLock::new(|| Mutex::new(String::new()));

/// Tynd host version — baked in at compile time from the workspace Cargo.toml.
const FRAMEWORK_VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn dispatch(method: &str, args: &Value) -> Result<Value, String> {
    match method {
        "getName" => Ok(Value::String(get_name())),
        "getVersion" => Ok(Value::String(get_version())),
        "getFrameworkVersion" => Ok(Value::String(FRAMEWORK_VERSION.into())),
        "getBundleIdentifier" => Ok(get_bundle_id().map_or(Value::Null, Value::String)),
        "setInfo" => {
            if let Some(n) = args.get("name").and_then(Value::as_str) {
                set_name(n.to_string());
            }
            if let Some(v) = args.get("version").and_then(Value::as_str) {
                set_version(v.to_string());
            }
            if let Some(b) = args.get("bundleId").and_then(Value::as_str) {
                set_bundle_id(b.to_string());
            }
            Ok(Value::Null)
        },
        "exit" => {
            let code = args.get("code").and_then(Value::as_i64).unwrap_or(0) as i32;
            crate::cleanup::run();
            std::process::exit(code);
        },
        "relaunch" => relaunch(),
        _ => Err(IpcError::unknown_method("app", method).into_string()),
    }
}

fn set_bundle_id(id: String) {
    *BUNDLE_ID.lock() = id;
}

fn get_bundle_id() -> Option<String> {
    let v = BUNDLE_ID.lock();
    if v.is_empty() {
        None
    } else {
        Some(v.clone())
    }
}

fn set_name(name: String) {
    *APP_NAME.lock() = name;
}

fn set_version(version: String) {
    *APP_VERSION.lock() = version;
}

fn get_name() -> String {
    let v = APP_NAME.lock();
    if !v.is_empty() {
        return v.clone();
    }
    drop(v);
    // Fallback: filename of the running binary without extension.
    std::env::current_exe()
        .ok()
        .and_then(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "tynd".into())
}

fn get_version() -> String {
    let v = APP_VERSION.lock();
    if v.is_empty() {
        "0.0.0".into()
    } else {
        v.clone()
    }
}

fn relaunch() -> Result<Value, String> {
    let exe = std::env::current_exe()
        .map_err(|e| String::from(IpcError::new("app", "relaunch", format!("current_exe: {e}"))))?;
    std::process::Command::new(&exe)
        .spawn()
        .map_err(|e| String::from(IpcError::new("app", "relaunch", format!("spawn: {e}"))))?;
    std::thread::spawn(|| {
        // Small delay so the child process is actually underway before we exit.
        std::thread::sleep(Duration::from_millis(100));
        crate::cleanup::run();
        std::process::exit(0);
    });
    Ok(Value::Null)
}
