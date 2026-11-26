//! System-tray stubs for builds without `api-tray`. Mirrors the crate-private
//! surface of `tray.rs` so the event loop and bootstrap compile unchanged and
//! callers get the standard `api_disabled` error instead of a missing symbol.

use serde_json::Value;
use tao::{event_loop::EventLoopProxy, window::Window};

use tynd_protocol::runtime::TrayConfig;

use crate::app::UserEvent;

/// Placeholder for the tray handle in builds without `api-tray`.
pub(crate) type SystemTray = ();

/// Error returned when a renderer calls `tray.*` and no tray is available.
pub(crate) fn unavailable_error() -> String {
    crate::api::api_disabled("tray")
}

/// Same shape as the `api-tray` build; the windows are unused here.
#[allow(dead_code)]
pub(crate) struct TrayCall<'a> {
    pub(crate) caller: &'a str,
    pub(crate) caller_origin: Option<&'a str>,
    pub(crate) window_for: &'a dyn Fn(&str) -> Option<&'a Window>,
}

/// A configured `tray` already failed startup in `ensure_static_apis_compiled`.
pub(crate) fn init(_config: Option<&TrayConfig>, _proxy: &EventLoopProxy<UserEvent>) {}

// Mirrors the signature of the real `tray::dispatch(&SystemTray, ..)`.
#[allow(clippy::trivially_copy_pass_by_ref)]
pub(crate) fn dispatch(
    _tray: &SystemTray,
    _call: &TrayCall<'_>,
    _method: &str,
    _args: &Value,
) -> Result<Value, String> {
    Err(unavailable_error())
}

pub(crate) fn toggle_attached<'w>(
    _tray: &str,
    _anchor: [f64; 4],
    _window_for: &dyn Fn(&str) -> Option<&'w Window>,
) {
}

pub(crate) fn on_window_blurred(_label: &str, _window: Option<&Window>) {}

/// No tray menus: a menu click never belongs to a tray.
#[allow(dead_code)]
pub(crate) fn on_menu_activated(_item: &str) -> Vec<String> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn tray_dispatch_reports_disabled_api() {
        let call = TrayCall {
            caller: "main",
            caller_origin: None,
            window_for: &|_| None,
        };
        let error = dispatch(&(), &call, "setTooltip", &json!({})).unwrap_err();
        assert!(error.contains("`api-tray`"), "unexpected error: {error}");
        assert_eq!(error, unavailable_error());
    }
}
