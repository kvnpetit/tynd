//! Native menu stubs for builds without `api-menu`. Mirrors the crate-private
//! surface of `menu.rs` so the event loop and bootstrap compile unchanged and
//! callers get the standard `api_disabled` error instead of a missing symbol.

use serde_json::Value;

// The tray menu is built with the same item builders, so `api-tray` alone
// still compiles them (and `muda`); the menu bar and context menus stay out.
#[cfg(feature = "api-tray")]
#[path = "menu/builder.rs"]
pub(crate) mod builder;

/// Placeholder for the application menu bar in builds without `api-menu`.
pub(crate) type AppMenu = ();

pub(crate) fn dispatch(_method: &str, _args: &Value) -> Result<Value, String> {
    Err(crate::api::api_disabled("menu"))
}

pub(crate) fn show_context_menu(
    _window: &tao::window::Window,
    _args: &Value,
) -> Result<(), String> {
    Err(crate::api::api_disabled("menu"))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn menu_dispatch_reports_disabled_api() {
        let error = dispatch("setItemEnabled", &json!({})).unwrap_err();
        assert!(error.contains("`api-menu`"), "unexpected error: {error}");
    }
}
