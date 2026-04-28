//! Tiny utilities shared between `app::run_app` and the IPC dispatcher.

use std::path::PathBuf;

use serde_json::Value;

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
    // `slug::slugify` returns a filesystem-safe ASCII slug with the same
    // shape the hand-rolled sanitizer produced (alphanumerics + dashes,
    // no leading/trailing separators). Falls back to `"tynd"` when the
    // app name strips down to an empty string — same default the inline
    // sanitizer carried.
    let slug = slug::slugify(app_name);
    let safe = if slug.is_empty() {
        "tynd".to_string()
    } else {
        slug
    };
    dirs::data_local_dir().map(|d| d.join(safe).join("WebView2"))
}
