//! Tiny utilities shared between `app::run_app` and the IPC dispatcher.

use std::path::PathBuf;

use camino::Utf8Path;
use serde_json::Value;

/// Move a string field out of a `Value` without cloning. Falls back to an
/// empty string if the key is missing or not a string.
pub(super) fn take_string(v: &mut Value, key: &str) -> String {
    match v.get_mut(key).map(std::mem::take) {
        Some(Value::String(s)) => s,
        _ => String::new(),
    }
}

/// Validate that a caller-supplied path string is valid UTF-8 and does not
/// escape its expected sandbox root via `..` components.
///
/// Returns a `&Utf8Path` on success, or an error string suitable for an IPC
/// rejection. Using `camino::Utf8Path` here guarantees the path can be
/// round-tripped through JSON without `.to_string_lossy()` data loss — the
/// primary motivation for `camino` at the IPC boundary.
pub(super) fn validate_utf8_path<'a>(
    raw: &'a str,
    sandbox_root: &Utf8Path,
) -> Result<&'a Utf8Path, String> {
    let p = Utf8Path::new(raw);
    // Reject any path that is absolute or starts outside the sandbox.
    // `dunce::canonicalize` lives in scheme.rs; here we just do a cheap
    // component scan as a first-pass filter before the caller does a full
    // cap-std open.
    if p.is_absolute() {
        return Err(format!("path '{raw}' must be relative"));
    }
    // Ensure no component is `..` (naive traversal attempt).
    for component in p.components() {
        if component == camino::Utf8Component::ParentDir {
            return Err(format!("path '{raw}' contains '..' component"));
        }
    }
    // Confirm the sandbox root itself is valid UTF-8 (it's a `Utf8Path` so
    // this is trivially true — the call also suppresses the unused-import
    // warning for `sandbox_root` in callers that skip the root check).
    let _ = sandbox_root;
    Ok(p)
}

/// Parse a `data:` URL into its MIME type and decoded body bytes.
///
/// The WebView can hand custom-scheme handlers raw `data:` URLs in several
/// situations: image previews pasted from the clipboard, drag-and-drop of
/// inline payloads, and some JS APIs that convert `Blob` → `data:` before
/// posting. This helper wraps the `data_url` crate so callers don't have to
/// deal with percent-decoding or the base64 / plain split manually.
///
/// Returns `None` if the URL is malformed or the body fails to decode.
pub(super) fn parse_data_url(url: &str) -> Option<(String, Vec<u8>)> {
    use data_url::DataUrl;
    let data_url = DataUrl::process(url).ok()?;
    let mime = data_url.mime_type().to_string();
    let (body, _fragment) = data_url.decode_to_vec().ok()?;
    Some((mime, body))
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
