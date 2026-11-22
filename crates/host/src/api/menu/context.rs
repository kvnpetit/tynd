//! Popup context-menu parsing, construction, positioning, and OS display.

use muda::ContextMenu as _;
use serde_json::Value;

use super::builder::fill;
use tynd_protocol::runtime::MenuItemDef;
/// Display `items` as a popup context menu anchored on `window`. Called from
/// the main thread (WindowCmd path); `x` / `y` are window-relative logical
/// pixels, `None` falls back to the cursor position. Clicks emit the usual
/// `menu:action` event — apps discriminate by item `id`.
pub(crate) fn show_context_menu(window: &tao::window::Window, args: &Value) -> Result<(), String> {
    let raw = args
        .get("items")
        .and_then(Value::as_array)
        .ok_or_else(|| "menu.showContextMenu: 'items' must be an array".to_string())?;
    let items: Vec<MenuItemDef> = serde_json::from_value(Value::Array(raw.clone()))
        .map_err(|e| format!("menu.showContextMenu: invalid items — {e}"))?;

    let menu = muda::Menu::new();
    fill(&|i: &dyn muda::IsMenuItem| menu.append(i), &items)?;

    let position: Option<dpi::Position> = match (
        args.get("x").and_then(Value::as_f64),
        args.get("y").and_then(Value::as_f64),
    ) {
        (Some(x), Some(y)) => Some(dpi::LogicalPosition::new(x, y).into()),
        _ => None,
    };

    show_on_window(window, &menu, position)
}

#[cfg(target_os = "windows")]
fn show_on_window(
    window: &tao::window::Window,
    menu: &muda::Menu,
    position: Option<dpi::Position>,
) -> Result<(), String> {
    use tao::platform::windows::WindowExtWindows;
    // SAFETY: `window.hwnd()` returns a valid HWND owned by the tao
    // window, and the window outlives this call. muda's API is safe to
    // invoke from the main thread, which is where wry's event loop
    // (and therefore this callback) already runs.
    unsafe {
        menu.show_context_menu_for_hwnd(window.hwnd(), position);
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn show_on_window(
    window: &tao::window::Window,
    menu: &muda::Menu,
    position: Option<dpi::Position>,
) -> Result<(), String> {
    use tao::platform::macos::WindowExtMacOS;
    let ns_view = window.ns_view();
    // SAFETY: `window.ns_view()` returns a valid NSView pointer owned by
    // tao; the view outlives this call. Cocoa menu APIs must be called
    // on the main thread, which is where this handler runs.
    unsafe {
        menu.show_context_menu_for_nsview(ns_view.cast(), position);
    }
    Ok(())
}

#[cfg(all(unix, not(target_os = "macos")))]
fn show_on_window(
    window: &tao::window::Window,
    menu: &muda::Menu,
    position: Option<dpi::Position>,
) -> Result<(), String> {
    use gtk::glib::object::Cast as _;
    use tao::platform::unix::WindowExtUnix;
    // muda wants `&gtk::Window`; tao hands us `&gtk::ApplicationWindow`,
    // a subclass — upcast_ref is zero-cost.
    let gtk_win: &gtk::Window = window.gtk_window().upcast_ref();
    menu.show_context_menu_for_gtk_window(gtk_win, position);
    Ok(())
}
