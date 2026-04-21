//! App-level APIs: name / version / exit / relaunch.
//!
//! Name + version are set by the backend on startup via `app.setInfo`.
//! The frontend then queries via `app.getName` / `app.getVersion`.
//! Exit + relaunch are uniform across runtimes.

use parking_lot::Mutex;
use serde_json::Value;
use std::sync::OnceLock;
use std::time::Duration;

static APP_NAME: OnceLock<Mutex<String>> = OnceLock::new();
static APP_VERSION: OnceLock<Mutex<String>> = OnceLock::new();
static BUNDLE_ID: OnceLock<Mutex<String>> = OnceLock::new();

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
        _ => Err(format!("app.{method}: unknown method")),
    }
}

fn set_bundle_id(id: String) {
    match BUNDLE_ID.get() {
        Some(m) => *m.lock() = id,
        None => {
            let _ = BUNDLE_ID.set(Mutex::new(id));
        },
    }
}

fn get_bundle_id() -> Option<String> {
    BUNDLE_ID.get().map(|m| m.lock().clone())
}

fn set_name(name: String) {
    match APP_NAME.get() {
        Some(m) => *m.lock() = name,
        None => {
            let _ = APP_NAME.set(Mutex::new(name));
        },
    }
}

fn set_version(version: String) {
    match APP_VERSION.get() {
        Some(m) => *m.lock() = version,
        None => {
            let _ = APP_VERSION.set(Mutex::new(version));
        },
    }
}

fn get_name() -> String {
    if let Some(m) = APP_NAME.get() {
        return m.lock().clone();
    }
    // Fallback: filename of the running binary without extension.
    std::env::current_exe()
        .ok()
        .and_then(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "tynd".into())
}

fn get_version() -> String {
    APP_VERSION
        .get()
        .map_or_else(|| "0.0.0".into(), |m| m.lock().clone())
}

fn relaunch() -> Result<Value, String> {
    let exe = std::env::current_exe().map_err(|e| format!("app.relaunch: current_exe: {e}"))?;
    std::process::Command::new(&exe)
        .spawn()
        .map_err(|e| format!("app.relaunch: spawn: {e}"))?;
    std::thread::spawn(|| {
        // Small delay so the child process is actually underway before we exit.
        std::thread::sleep(Duration::from_millis(100));
        crate::cleanup::run();
        std::process::exit(0);
    });
    Ok(Value::Null)
}
