//! Application menu-bar construction, registry-backed mutations, and OS initialization.

use std::cell::RefCell;

use serde_json::Value;

use super::{
    builder::{fill_with_registry, RegContainer},
    registry::{MenuRegistry, Scope},
};
use tynd_protocol::runtime::MenuItemDef;

use crate::ipc::error::IpcError;

thread_local! {
    /// The live application menu bar and its item registry (main thread, like
    /// tray menus: the `!Send` muda handles never reach a global static). The
    /// event loop also holds the root when the bar came from the config;
    /// `setMenu` refills it in place.
    static BAR: RefCell<Option<MenuRegistry>> = const { RefCell::new(None) };
}

/// Build the application menu bar from the config. Only submenus are valid
/// top-level entries (macOS requires it); other entries are skipped.
pub(crate) fn build_bar(items: &[MenuItemDef]) -> Result<muda::Menu, String> {
    let submenus: Vec<MenuItemDef> = items
        .iter()
        .filter(|item| matches!(item, MenuItemDef::Submenu(_)))
        .cloned()
        .collect();
    let registry = MenuRegistry::build(Scope::BAR, &submenus)?;
    let root = registry.root().clone();
    BAR.with(|bar| *bar.borrow_mut() = Some(registry));
    Ok(root)
}

/// Runtime menu-bar mutations. Must run on the main thread — wired through
/// `UserEvent::MenuCmd` so muda's `!Send` handles never leave the event-loop
/// thread. Returns `Ok(Null)` on every successful mutation; the actual
/// state lives in the registry + muda items. `primary` is the main window,
/// used when `setMenu` has to attach a bar that did not exist yet.
pub(crate) fn dispatch(
    primary: Option<&tao::window::Window>,
    method: &str,
    args: &Value,
) -> Result<Value, String> {
    if method == "setMenu" {
        set_menu(primary, args)?;
        return Ok(Value::Null);
    }
    BAR.with(|bar| {
        let mut bar = bar.borrow_mut();
        let result = match bar.as_mut() {
            Some(registry) => registry.dispatch(method, args),
            None => is_item_method(method).then(|| Err(no_bar_error(method))),
        };
        match result {
            Some(result) => result.map(|()| Value::Null),
            None => Err(IpcError::unknown_method("menu", method).into_string()),
        }
    })
}

fn is_item_method(method: &str) -> bool {
    matches!(
        method,
        "update"
            | "setItemEnabled"
            | "setItemLabel"
            | "setItemChecked"
            | "setItemVisible"
            | "setItemAccelerator"
            | "setItemIcon"
            | "append"
            | "remove"
    )
}

fn no_bar_error(op: &str) -> String {
    format!(
        "menu.{op}: no application menu — set `menu` in app.start or call menu.setMenu(items) first"
    )
}

/// Click hook (main thread, from muda's event handler): keeps the bar's radio
/// groups exclusive. Returns whether the click came from the bar.
pub(crate) fn on_item_activated(id: &str) -> bool {
    BAR.try_with(|bar| {
        // A click can be delivered while a popup runs inside a dispatch call
        // (macOS menus are modal); never panic on a held borrow.
        bar.try_borrow()
            .ok()
            .and_then(|bar| bar.as_ref().map(|registry| registry.on_activated(id)))
            .unwrap_or(false)
    })
    .unwrap_or(false)
}

/// `setMenu({ items })`: replace the whole bar. Attaches a bar to the main
/// window when the app started without one.
fn set_menu(primary: Option<&tao::window::Window>, args: &Value) -> Result<(), String> {
    let items = super::window_bar::parse_bar_items("menu.setMenu", args)?
        .ok_or_else(|| "menu.setMenu: 'items' must be an array".to_string())?;
    // Build a throwaway copy first so a bad icon or id leaves the live bar
    // untouched.
    let scratch = muda::Menu::new();
    fill_with_registry(&RegContainer::Root(scratch), &items, &mut |_, _| {})
        .map_err(|e| format!("menu.setMenu: {e}"))?;

    let existing = BAR
        .with(|bar| bar.borrow_mut().take())
        .map(|bar| bar.root().clone());
    let root = if let Some(root) = existing {
        root
    } else {
        let window =
            primary.ok_or_else(|| "menu.setMenu: the main window is not available".to_string())?;
        let root = muda::Menu::new();
        init_bar(&root, window);
        root
    };
    while root.remove_at(0).is_some() {}
    let registry =
        MenuRegistry::fill(Scope::BAR, root, &items).map_err(|e| format!("menu.setMenu: {e}"))?;
    BAR.with(|bar| *bar.borrow_mut() = Some(registry));
    Ok(())
}

/// Detach the application bar from the main window (`win.setMenu(null)`);
/// a later `setMenu` attaches a fresh one.
pub(super) fn remove_bar(primary: &tao::window::Window) {
    let Some(registry) = BAR.with(|bar| bar.borrow_mut().take()) else {
        return;
    };
    let root = registry.root();
    #[cfg(target_os = "macos")]
    root.remove_for_nsapp();
    super::window_bar::detach(root, primary);
}

pub(crate) fn init_bar(menu: &muda::Menu, window: &tao::window::Window) {
    #[cfg(target_os = "windows")]
    {
        use tao::platform::windows::WindowExtWindows;
        // SAFETY: `window.hwnd()` is a valid HWND that lives at least as
        // long as `window`; muda stores it for later use and the window
        // stays alive for the lifetime of the app.
        unsafe {
            if let Err(e) = menu.init_for_hwnd(window.hwnd()) {
                crate::tynd_warn!("menu.init_for_hwnd: {e}");
            }
        }
    }
    #[cfg(target_os = "macos")]
    {
        let _ = window;
        menu.init_for_nsapp();
    }
    #[cfg(target_os = "linux")]
    {
        use tao::platform::unix::WindowExtUnix;
        // Must run before the webview container is packed into the default
        // vbox (`app::linux_webview`): muda inserts the bar at index 0 of the
        // box, and the container then fills the remaining content area.
        if let Err(e) = menu.init_for_gtk_window(window.gtk_window(), window.default_vbox()) {
            crate::tynd_warn!("menu.init_for_gtk_window: {e}");
        }
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        let _ = (menu, window);
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn bar(items: Value) {
        let items: Vec<MenuItemDef> = serde_json::from_value(items).unwrap();
        build_bar(&items).unwrap();
    }

    fn with_bar<T>(f: impl FnOnce(&MenuRegistry) -> T) -> T {
        BAR.with(|bar| f(bar.borrow().as_ref().expect("a bar is built")))
    }

    fn checked(id: &str) -> bool {
        with_bar(|bar| bar.is_checked(id)).unwrap_or_else(|| panic!("{id} is not a check item"))
    }

    fn registered(id: &str) -> bool {
        with_bar(|bar| bar.contains(id))
    }

    fn view_menu() -> Value {
        json!([{
            "type": "submenu", "id": "view", "label": "View",
            "items": [
                { "type": "radio", "id": "small", "label": "Small", "checked": true },
                { "type": "radio", "id": "large", "label": "Large" },
                { "type": "separator" },
                { "type": "radio", "id": "light", "label": "Light", "checked": true },
                { "type": "item", "id": "reload", "label": "Reload", "accelerator": "CmdOrCtrl+R" }
            ]
        }])
    }

    #[test]
    fn radio_groups_stop_at_separators() {
        bar(view_menu());
        dispatch(
            None,
            "setItemChecked",
            &json!({ "id": "large", "checked": true }),
        )
        .unwrap();
        assert!(!checked("small"));
        assert!(checked("large"));
        assert!(checked("light"), "a separator ends the radio group");

        with_bar(|bar| bar.click("small"));
        assert!(checked("small"));
        assert!(!checked("large"));
    }

    #[test]
    fn append_insert_and_remove_items() {
        bar(view_menu());
        dispatch(
            None,
            "append",
            &json!({ "parent": "view", "index": 0, "item": { "type": "item", "id": "zoom", "label": "Zoom" } }),
        )
        .unwrap();
        assert!(registered("zoom"));

        let duplicate = dispatch(
            None,
            "append",
            &json!({ "parent": "view", "item": { "type": "item", "id": "zoom", "label": "Zoom" } }),
        )
        .unwrap_err();
        assert!(duplicate.contains("already exists"), "{duplicate}");

        let out_of_range = dispatch(
            None,
            "append",
            &json!({ "parent": "view", "index": 99, "item": { "type": "item", "id": "z2", "label": "Z" } }),
        )
        .unwrap_err();
        assert!(out_of_range.contains("out of range"), "{out_of_range}");

        let top_level_item = dispatch(
            None,
            "append",
            &json!({ "item": { "type": "item", "id": "z3", "label": "Z" } }),
        )
        .unwrap_err();
        assert!(
            top_level_item.contains("must be submenus"),
            "{top_level_item}"
        );

        dispatch(None, "remove", &json!({ "id": "view" })).unwrap();
        assert!(!registered("view"));
        assert!(
            !registered("zoom"),
            "descendants are forgotten with their submenu"
        );
        assert!(!registered("small"));
    }

    #[test]
    fn accelerator_and_icon_mutations() {
        bar(view_menu());
        let bad = dispatch(
            None,
            "setItemAccelerator",
            &json!({ "id": "reload", "accelerator": "Ctrl+Nope+" }),
        )
        .unwrap_err();
        assert!(bad.contains("invalid accelerator"), "{bad}");
        dispatch(
            None,
            "setItemAccelerator",
            &json!({ "id": "reload", "accelerator": "F5" }),
        )
        .unwrap();

        dispatch(
            None,
            "setItemIcon",
            &json!({ "id": "reload", "nativeIcon": "refresh" }),
        )
        .unwrap();
        with_bar(|bar| {
            assert!(bar.is_icon_item("reload"));
            assert_eq!(bar.accelerator("reload"), Some("F5"));
        });

        let radio_icon = dispatch(
            None,
            "setItemIcon",
            &json!({ "id": "small", "nativeIcon": "add" }),
        )
        .unwrap_err();
        assert!(radio_icon.contains("cannot show an icon"), "{radio_icon}");
    }

    #[test]
    fn set_menu_validates_before_replacing() {
        bar(view_menu());
        let error = dispatch(
            None,
            "setMenu",
            &json!({ "items": [{ "type": "item", "id": "x", "label": "X" }] }),
        )
        .unwrap_err();
        assert!(error.contains("must be a submenu"), "{error}");

        let bad_icon = dispatch(
            None,
            "setMenu",
            &json!({ "items": [{ "type": "submenu", "label": "F", "items": [
                { "type": "item", "id": "x", "label": "X", "nativeIcon": "nope" }
            ] }] }),
        )
        .unwrap_err();
        assert!(bad_icon.contains("unknown native icon"), "{bad_icon}");
        assert!(
            registered("reload"),
            "a failed setMenu leaves the bar untouched"
        );

        dispatch(
            None,
            "setMenu",
            &json!({ "items": [{ "type": "submenu", "id": "file", "label": "File", "items": [] }] }),
        )
        .unwrap();
        assert!(registered("file"));
        assert!(!registered("reload"));
    }
}
