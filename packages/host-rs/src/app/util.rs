//! Tiny utilities shared between `app::run_app` and the IPC dispatcher.

use serde_json::Value;
use std::path::PathBuf;

/// Move a string field out of a `Value` without cloning. Falls back to an
/// empty string if the key is missing or not a string.
pub(super) fn take_string(v: &mut Value, key: &str) -> String {
    match v.get_mut(key).map(std::mem::take) {
        Some(Value::String(s)) => s,
        _ => String::new(),
    }
}

/// Return an OS-appropriate directory for WebView persistent data.
/// Windows : %LOCALAPPDATA%\<app>\WebView2
/// macOS   : ~/Library/Application Support/<app>/WebView2
/// Linux   : ~/.local/share/<app>/webview
pub(super) fn webview_data_dir(app_name: &str) -> Option<PathBuf> {
    // Sanitize app name: keep [A-Za-z0-9_], collapse other chars to single dashes.
    let mut safe = String::new();
    let mut prev_dash = true; // start true to suppress leading dashes
    for c in app_name.chars() {
        if c.is_alphanumeric() || c == '_' {
            safe.push(c);
            prev_dash = false;
        } else if !prev_dash {
            safe.push('-');
            prev_dash = true;
        }
    }
    while safe.ends_with('-') {
        safe.pop();
    }
    if safe.is_empty() {
        safe = "tynd".to_string();
    }
    dirs::data_local_dir().map(|d| d.join(safe).join("WebView2"))
}
