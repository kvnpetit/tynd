//! Popup context menus: parsing, the registry behind declarative
//! `data-tynd-contextmenu` menus, positioning, and OS display.

use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    sync::Mutex,
};

use muda::ContextMenu as _;
use serde_json::{json, Value};

use super::builder::{fill_with_registry, RegContainer};
use tynd_protocol::runtime::MenuItemDef;

/// Longest accepted context-menu id (and registry key).
const MAX_MENU_ID_LEN: usize = 128;
/// Most context menus one app may register.
const MAX_REGISTERED_MENUS: usize = 256;
/// Longest accepted `data-tynd-contextmenu-data` payload, in bytes.
const MAX_CONTEXT_DATA_LEN: usize = 4096;

thread_local! {
    /// Main-thread registry of named context menus (`menu.registerContextMenu`).
    /// Stores the definitions, not muda handles: each popup builds a fresh
    /// menu, so a registered menu can be shown in any window.
    static REGISTERED: RefCell<HashMap<String, Vec<MenuItemDef>>> = RefCell::new(HashMap::new());
}

/// The last popup opened from a registered menu. A click on one of its item
/// ids carries `contextMenu` and `data` in the `menu:action` event. Kept
/// until the next popup because the click can be delivered after the popup
/// call returns (Windows posts `WM_COMMAND`, GTK pops up asynchronously).
struct ActivePopup {
    menu: String,
    data: Value,
    ids: HashSet<String>,
}

static ACTIVE_POPUP: Mutex<Option<ActivePopup>> = Mutex::new(None);

fn parse_items(op: &str, raw: Option<&Value>) -> Result<Vec<MenuItemDef>, String> {
    let raw = raw
        .filter(|items| items.is_array())
        .ok_or_else(|| format!("menu.{op}: 'items' must be an array"))?;
    serde_json::from_value(raw.clone()).map_err(|e| format!("menu.{op}: invalid items — {e}"))
}

fn menu_id<'a>(op: &str, field: &str, args: &'a Value) -> Result<&'a str, String> {
    let id = args
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("menu.{op}: missing '{field}'"))?;
    if id.is_empty() || id.len() > MAX_MENU_ID_LEN || id == "none" || id == "default" {
        return Err(format!(
            "menu.{op}: '{field}' must be 1-{MAX_MENU_ID_LEN} characters and not \"none\" / \"default\" (reserved by data-tynd-contextmenu)"
        ));
    }
    Ok(id)
}

/// Build `items` into a fresh popup menu, returning it with the ids it holds.
fn build_popup(items: &[MenuItemDef]) -> Result<(muda::Menu, HashSet<String>), String> {
    let menu = muda::Menu::new();
    let mut ids = HashSet::new();
    fill_with_registry(&RegContainer::Root(menu.clone()), items, &mut |id, _| {
        ids.insert(id);
    })?;
    Ok((menu, ids))
}

fn position(args: &Value) -> Option<dpi::Position> {
    match (
        args.get("x").and_then(Value::as_f64),
        args.get("y").and_then(Value::as_f64),
    ) {
        (Some(x), Some(y)) if x.is_finite() && y.is_finite() => {
            Some(dpi::LogicalPosition::new(x, y).into())
        },
        _ => None,
    }
}

/// Display `items` as a popup context menu anchored on `window`. Called from
/// the main thread (WindowCmd path); `x` / `y` are window-relative logical
/// pixels, `None` falls back to the cursor position. Clicks emit the usual
/// `menu:action` event — apps discriminate by item `id`.
pub(crate) fn show_context_menu(window: &tao::window::Window, args: &Value) -> Result<(), String> {
    let items = parse_items("showContextMenu", args.get("items"))?;
    let (menu, _) = build_popup(&items)?;
    // An anonymous popup carries no data: drop the last registered one so its
    // ids don't pick up stale `data`.
    set_active_popup(None);
    show_on_window(window, &menu, position(args))
}

/// `menu.registerContextMenu(id, items)`: validate and store a named menu.
pub(crate) fn register(args: &Value) -> Result<Value, String> {
    let id = menu_id("registerContextMenu", "id", args)?;
    let items = parse_items("registerContextMenu", args.get("items"))?;
    // Build once so a bad icon / unknown native icon fails here, not on the
    // first right-click.
    build_popup(&items).map_err(|e| format!("menu.registerContextMenu: {e}"))?;
    REGISTERED.with(|registry| {
        let mut registry = registry.borrow_mut();
        if !registry.contains_key(id) && registry.len() >= MAX_REGISTERED_MENUS {
            return Err(format!(
                "menu.registerContextMenu: at most {MAX_REGISTERED_MENUS} context menus can be registered"
            ));
        }
        registry.insert(id.to_string(), items);
        Ok(())
    })?;
    Ok(Value::Null)
}

/// `menu.unregisterContextMenu(id)`. Unknown ids are a no-op.
pub(crate) fn unregister(args: &Value) -> Result<Value, String> {
    let id = menu_id("unregisterContextMenu", "id", args)?;
    REGISTERED.with(|registry| registry.borrow_mut().remove(id));
    Ok(Value::Null)
}

/// Pop up a registered menu on `window` (declarative `data-tynd-contextmenu`
/// right-click, or `menu.popupContextMenu`). `data` rides along with the
/// resulting `menu:action` event.
pub(crate) fn popup_registered(
    window: Option<&tao::window::Window>,
    args: &Value,
) -> Result<Value, String> {
    let id = menu_id("popupContextMenu", "menu", args)?;
    let data = match args.get("data") {
        None | Some(Value::Null) => Value::Null,
        Some(Value::String(data)) if data.len() <= MAX_CONTEXT_DATA_LEN => {
            Value::String(data.clone())
        },
        Some(_) => {
            return Err(format!(
                "menu.popupContextMenu: 'data' must be a string of at most {MAX_CONTEXT_DATA_LEN} bytes"
            ))
        },
    };
    let window =
        window.ok_or_else(|| "menu.popupContextMenu: the calling window is gone".to_string())?;
    let items = REGISTERED
        .with(|registry| registry.borrow().get(id).cloned())
        .ok_or_else(|| {
            format!(
                "menu.popupContextMenu: no context menu registered as '{id}' — call menu.registerContextMenu(\"{id}\", items) first"
            )
        })?;
    let (menu, ids) = build_popup(&items)?;
    set_active_popup(Some(ActivePopup {
        menu: id.to_string(),
        data,
        ids,
    }));
    show_on_window(window, &menu, position(args))?;
    Ok(Value::Null)
}

fn set_active_popup(popup: Option<ActivePopup>) {
    let mut active = ACTIVE_POPUP
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    *active = popup;
}

/// `menu:action` payload for a click on `id`: `{ id }`, plus `contextMenu`
/// and `data` when the item belongs to the last registered popup.
pub(crate) fn action_payload(id: &str) -> Value {
    let active = ACTIVE_POPUP
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    match active.as_ref() {
        Some(popup) if popup.ids.contains(id) => {
            json!({ "id": id, "contextMenu": popup.menu, "data": popup.data })
        },
        _ => json!({ "id": id }),
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_ids_reject_reserved_and_oversized_values() {
        for bad in ["", "none", "default"] {
            let error = menu_id("registerContextMenu", "id", &json!({ "id": bad })).unwrap_err();
            assert!(error.contains("reserved"), "{error}");
        }
        let long = "x".repeat(MAX_MENU_ID_LEN + 1);
        assert!(menu_id("registerContextMenu", "id", &json!({ "id": long })).is_err());
        assert_eq!(
            menu_id("registerContextMenu", "id", &json!({ "id": "row" })),
            Ok("row")
        );
    }

    #[test]
    fn items_must_be_a_valid_array() {
        assert!(parse_items("registerContextMenu", Some(&json!({})))
            .unwrap_err()
            .contains("must be an array"));
        assert!(
            parse_items("registerContextMenu", Some(&json!([{ "type": "bogus" }])))
                .unwrap_err()
                .contains("invalid items")
        );
    }

    #[test]
    fn popup_rejects_bad_data_before_touching_the_window() {
        let error = popup_registered(None, &json!({ "menu": "row", "data": 1 })).unwrap_err();
        assert!(error.contains("'data' must be a string"), "{error}");
        let long = "x".repeat(MAX_CONTEXT_DATA_LEN + 1);
        assert!(popup_registered(None, &json!({ "menu": "row", "data": long })).is_err());
    }

    #[test]
    fn position_needs_both_finite_coordinates() {
        assert!(position(&json!({ "x": 1.0 })).is_none());
        assert!(position(&json!({ "x": 1.0, "y": 2.0 })).is_some());
    }

    #[test]
    fn action_payload_adds_popup_data_only_for_its_items() {
        set_active_popup(Some(ActivePopup {
            menu: "row".into(),
            data: json!("42"),
            ids: HashSet::from(["row.delete".to_string()]),
        }));
        assert_eq!(
            action_payload("row.delete"),
            json!({ "id": "row.delete", "contextMenu": "row", "data": "42" })
        );
        assert_eq!(action_payload("file.save"), json!({ "id": "file.save" }));
        set_active_popup(None);
    }
}
