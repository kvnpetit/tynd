//! Tiny utilities for `app::run_app`: WebView data directories.

use std::path::PathBuf;

/// Return an OS-appropriate directory for WebView persistent data.
/// Windows : %LOCALAPPDATA%\<app>\WebView2
/// macOS   : ~/Library/Application Support/<app>/WebView2
/// Linux   : ~/.local/share/<app>/webview
pub(super) fn webview_data_dir(app_name: &str) -> Option<PathBuf> {
    // Falls back to `"tynd"` when the app name slugifies to an empty
    // string so a directory always exists.
    dirs::data_local_dir().map(|d| d.join(app_data_slug(app_name)).join("WebView2"))
}

/// Persistent data directory for a named secondary-webview partition.
///
/// `partition` is validated before this helper is called, so joining it cannot
/// escape the app-owned WebView directory.
pub(super) fn webview_partition_data_dir(app_name: &str, partition: &str) -> Option<PathBuf> {
    webview_data_dir(app_name).map(|root| root.join("Partitions").join(partition))
}

/// Stable 128-bit identifier used by WKWebView's named data stores.
#[cfg(target_os = "macos")]
pub(super) fn webview_partition_identifier(app_name: &str, partition: &str) -> [u8; 16] {
    let digest = blake3::hash(format!("{app_name}\0{partition}").as_bytes());
    let mut identifier = [0_u8; 16];
    identifier.copy_from_slice(&digest.as_bytes()[..16]);
    identifier
}

pub(super) fn validate_data_partition(value: &str) -> Result<String, String> {
    let bytes = value.as_bytes();
    if bytes.is_empty() || bytes.len() > 64 {
        return Err("dataPartition must contain 1..=64 ASCII characters".into());
    }
    if !bytes[0].is_ascii_alphanumeric()
        || !bytes
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(
            "dataPartition must start with a letter or digit and contain only ASCII letters, digits, '.', '-' or '_'"
                .into(),
        );
    }
    Ok(value.to_ascii_lowercase())
}

fn app_data_slug(app_name: &str) -> String {
    let slug = crate::util::slug_ascii(app_name);
    if slug.is_empty() {
        "tynd".to_string()
    } else {
        slug
    }
}
