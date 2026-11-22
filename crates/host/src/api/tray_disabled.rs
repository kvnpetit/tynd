//! System-tray stubs for builds without `api-tray`. Mirrors the crate-private
//! surface of `tray.rs` so the event loop and bootstrap compile unchanged and
//! callers get the standard `api_disabled` error instead of a missing symbol.

use serde_json::Value;

/// Placeholder for the tray handle in builds without `api-tray`.
pub(crate) type SystemTray = ();

/// Error returned when a renderer calls `tray.*` and no tray is available.
pub(crate) fn unavailable_error() -> String {
    crate::api::api_disabled("tray")
}

// Mirrors the signature of the real `tray::dispatch(&TrayIcon, ..)`.
#[allow(clippy::trivially_copy_pass_by_ref)]
pub(crate) fn dispatch(_tray: &SystemTray, _method: &str, _args: &Value) -> Result<Value, String> {
    Err(unavailable_error())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn tray_dispatch_reports_disabled_api() {
        let error = dispatch(&(), "setTooltip", &json!({})).unwrap_err();
        assert!(error.contains("`api-tray`"), "unexpected error: {error}");
        assert_eq!(error, unavailable_error());
    }
}
