//! Native menu facade: shared builders, popup context menus, and application menu bar.
//!
//! This is the `api-menu` build of the module. Without the feature,
//! `menu_disabled.rs` supplies the same crate-private surface as stubs (and
//! only the `builder` submodule when `api-tray` still needs it), so the event
//! loop keeps one code path without linking `muda`.

mod bar;
pub(crate) mod builder;
mod context;
pub(crate) mod registry;
mod window_bar;

use serde_json::Value;

pub(crate) use bar::{build_bar, init_bar};
pub(crate) use context::show_context_menu;
pub(crate) use window_bar::{set_primary_window_menu, set_secondary_window_menu};

/// A secondary window's own menu bar (`win.setMenu`) and its item registry;
/// dropped with the window.
pub(crate) type WindowMenu = Option<std::rc::Rc<registry::MenuRegistry>>;

/// Handle that keeps the application menu bar alive for the event loop's lifetime.
pub(crate) type AppMenu = muda::Menu;

/// Injected into every webview: opens the context menu registered under an
/// element's `data-tynd-contextmenu` on right-click (`"none"` suppresses the
/// webview menu, `"default"` restores it below a suppressed ancestor).
pub(crate) const JS_CONTEXT_MENU: &str = include_str!("menu/context_menu.js");

/// Windows a menu command can target: the caller's window (context-menu
/// popups) and the main window (where `setMenu` attaches a new bar).
pub(crate) struct MenuTarget<'a> {
    pub(crate) source: Option<&'a tao::window::Window>,
    pub(crate) primary: Option<&'a tao::window::Window>,
}

/// Renderer `menu.*` calls, on the main thread (`UserEvent::MenuCmd`).
pub(crate) fn dispatch(
    target: &MenuTarget<'_>,
    method: &str,
    args: &Value,
) -> Result<Value, String> {
    match method {
        "registerContextMenu" => context::register(args),
        "unregisterContextMenu" => context::unregister(args),
        "popupContextMenu" => context::popup_registered(target.source, args),
        _ => bar::dispatch(target.primary, method, args),
    }
}

/// `menu:action` payload for a click on `id` (menu bar, tray or context
/// menu), and how many of the application bar, window bars, the Dock menu
/// and the active context menu claim the click (more than one: the id is
/// ambiguous, muda reports only the id). Also keeps the radio groups of the
/// menu the click came from exclusive.
pub(crate) fn action_payload(id: &str) -> (Value, usize) {
    let claims = usize::from(bar::on_item_activated(id)) + registry::activate_tracked(id);
    let payload = context::action_payload(id);
    let claims = claims + usize::from(payload.get("contextMenu").is_some());
    (payload, claims)
}
