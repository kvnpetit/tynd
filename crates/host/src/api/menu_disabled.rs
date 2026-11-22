//! Native menu stubs for builds without `api-menu`. Mirrors the crate-private
//! surface of `menu.rs` so the event loop and bootstrap compile unchanged and
//! callers get the standard `api_disabled` error instead of a missing symbol.

use serde_json::Value;

// The tray menu is built with the same item builders, so `api-tray` alone
// still compiles them (and `muda`); the menu bar and context menus stay out.
#[cfg(feature = "api-tray")]
#[path = "menu/builder.rs"]
pub(crate) mod builder;
#[cfg(feature = "api-tray")]
#[path = "menu/registry.rs"]
pub(crate) mod registry;

/// Placeholder for the application menu bar in builds without `api-menu`.
pub(crate) type AppMenu = ();

/// No declarative context menus without `api-menu`.
pub(crate) const JS_CONTEXT_MENU: &str = "";

/// Same shape as the `api-menu` build; the windows are unused here.
#[allow(dead_code)]
pub(crate) struct MenuTarget<'a> {
    pub(crate) source: Option<&'a tao::window::Window>,
    pub(crate) primary: Option<&'a tao::window::Window>,
}

pub(crate) fn dispatch(
    _target: &MenuTarget<'_>,
    _method: &str,
    _args: &Value,
) -> Result<Value, String> {
    Err(crate::api::api_disabled("menu"))
}

/// `menu:action` payload for a click on `id` (tray menus in `api-tray` builds).
#[cfg(feature = "api-tray")]
pub(crate) fn action_payload(id: &str) -> (Value, usize) {
    (serde_json::json!({ "id": id }), 0)
}

/// No per-window menu bars without `api-menu`.
pub(crate) type WindowMenu = ();

pub(crate) fn set_primary_window_menu(
    _primary: &tao::window::Window,
    _args: &Value,
) -> Result<(), String> {
    Err(crate::api::api_disabled("menu"))
}

pub(crate) fn set_secondary_window_menu(
    _window: &tao::window::Window,
    _slot: &mut WindowMenu,
    _args: &Value,
) -> Result<(), String> {
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
        let target = MenuTarget {
            source: None,
            primary: None,
        };
        let error = dispatch(&target, "setItemEnabled", &json!({})).unwrap_err();
        assert!(error.contains("`api-menu`"), "unexpected error: {error}");
    }
}
