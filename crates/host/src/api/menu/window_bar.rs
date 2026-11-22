//! Per-window menu bars (`win.setMenu(items)`). The primary window's bar is
//! the application bar (`bar.rs`, runtime-mutable by id); a secondary window
//! gets its own bar on Windows and Linux. macOS has one app-wide menu bar.

use std::rc::Rc;

use serde_json::Value;
use tao::window::Window;
use tynd_protocol::runtime::MenuItemDef;

use super::{
    bar::{self, init_bar},
    registry::{self, MenuRegistry, Scope},
    WindowMenu,
};

/// Parse `{ items: MenuItemDef[] | null }`: `None` removes the bar.
pub(super) fn parse_bar_items(op: &str, args: &Value) -> Result<Option<Vec<MenuItemDef>>, String> {
    let items = match args.get("items") {
        None | Some(Value::Null) => return Ok(None),
        Some(items) => serde_json::from_value::<Vec<MenuItemDef>>(items.clone())
            .map_err(|e| format!("{op}: 'items' must be an array of menu items: {e}"))?,
    };
    if let Some(index) = items
        .iter()
        .position(|item| !matches!(item, MenuItemDef::Submenu(_)))
    {
        return Err(format!(
            "{op}: items[{index}] must be a submenu — top-level bar entries are submenus ({{ type: \"submenu\", label, items }})"
        ));
    }
    Ok(Some(items))
}

/// `setMenu` on the primary window: replace (or remove) the application bar.
pub(crate) fn set_primary_window_menu(primary: &Window, args: &Value) -> Result<(), String> {
    if parse_bar_items("window.setMenu", args)?.is_some() {
        bar::dispatch(Some(primary), "setMenu", args).map(drop)
    } else {
        bar::remove_bar(primary);
        Ok(())
    }
}

/// `setMenu` on a secondary window. `slot` holds the window's current bar
/// and its own item registry; clicks emit `menu:action { id }` like
/// application-bar items, and its radio groups stay exclusive.
pub(crate) fn set_secondary_window_menu(
    window: &Window,
    slot: &mut WindowMenu,
    args: &Value,
) -> Result<(), String> {
    let items = parse_bar_items("window.setMenu", args)?;
    if cfg!(target_os = "macos") {
        return Err(
            "window.setMenu: not supported on macOS for secondary windows — the menu bar is \
             app-wide; use menu.setMenu"
                .into(),
        );
    }
    let next = items
        .map(|items| {
            MenuRegistry::build(Scope::BAR, &items)
                .map(Rc::new)
                .map_err(|e| format!("window.setMenu: {e}"))
        })
        .transpose()?;
    if let Some(previous) = slot.take() {
        detach(previous.root(), window);
    }
    if let Some(menu) = next {
        init_bar(menu.root(), window);
        registry::track(&menu);
        *slot = Some(menu);
    }
    Ok(())
}

pub(super) fn detach(menu: &muda::Menu, window: &Window) {
    #[cfg(target_os = "windows")]
    {
        use tao::platform::windows::WindowExtWindows;
        // SAFETY: `window.hwnd()` is the live HWND the bar was attached to.
        if let Err(e) = unsafe { menu.remove_for_hwnd(window.hwnd()) } {
            crate::tynd_warn!("menu.remove_for_hwnd: {e}");
        }
    }
    #[cfg(target_os = "linux")]
    {
        use tao::platform::unix::WindowExtUnix;
        if let Err(e) = menu.remove_for_gtk_window(window.gtk_window()) {
            crate::tynd_warn!("menu.remove_for_gtk_window: {e}");
        }
    }
    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    {
        let _ = (menu, window);
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn null_or_missing_items_remove_the_bar() {
        assert!(parse_bar_items("op", &json!({})).unwrap().is_none());
        assert!(parse_bar_items("op", &json!({ "items": null }))
            .unwrap()
            .is_none());
    }

    #[test]
    fn top_level_entries_must_be_submenus() {
        let items = json!({ "items": [
            { "type": "submenu", "label": "File", "items": [] },
            { "type": "item", "id": "x", "label": "X" }
        ]});
        let error = parse_bar_items("window.setMenu", &items).unwrap_err();
        assert!(error.contains("items[1]"), "{error}");
        let error = parse_bar_items("window.setMenu", &json!({ "items": 3 })).unwrap_err();
        assert!(error.contains("'items'"), "{error}");
    }
}
