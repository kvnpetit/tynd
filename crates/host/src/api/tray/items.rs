//! Menu of one tray icon: a shared [`MenuRegistry`] in the `tray` scope, so
//! the runtime mutations (`setItemEnabled`, `append`, `removeItem`, …) reach
//! the native items without a rebuild, and roles, icons and radio groups
//! behave like the application menu bar.

use serde_json::Value;

use tynd_protocol::runtime::MenuItemDef;

use crate::api::menu::registry::{self, MenuRegistry, Scope};

pub(super) type TrayMenu = MenuRegistry;

/// Build a fresh tray menu. A bad item (icon, accelerator…) fails the whole
/// build, so a caller replacing a live menu keeps the old one on error.
pub(super) fn build(items: &[MenuItemDef]) -> Result<TrayMenu, String> {
    MenuRegistry::build(Scope::TRAY, items)
}

pub(super) fn optional_field<T: serde::de::DeserializeOwned>(
    args: &Value,
    op: &str,
    field: &str,
) -> Result<Option<T>, String> {
    registry::optional_field("tray", args, op, field)
}

// AppKit requires the main thread; macOS exercises the tray in smoke-packaged.ts.
#[cfg(all(test, not(target_os = "macos")))]
mod tests {
    use serde_json::json;

    use super::*;

    fn menu(items: Value) -> TrayMenu {
        let items: Vec<MenuItemDef> = serde_json::from_value(items).unwrap();
        build(&items).unwrap()
    }

    fn checked(menu: &TrayMenu, id: &str) -> bool {
        menu.is_checked(id)
            .unwrap_or_else(|| panic!("{id} is not a check item"))
    }

    fn sample() -> TrayMenu {
        // No `type` on plain items: the protocol parse defaults it.
        menu(json!([
            { "type": "radio", "id": "small", "label": "Small", "checked": true },
            { "type": "radio", "id": "large", "label": "Large" },
            { "type": "separator" },
            { "type": "radio", "id": "light", "label": "Light", "checked": true },
            { "id": "show", "label": "Show", "accelerator": "CmdOrCtrl+S" },
            { "id": "more", "label": "More", "items": [{ "id": "about", "label": "About" }] }
        ]))
    }

    #[test]
    fn untyped_items_build_and_register() {
        let menu = sample();
        assert!(menu.contains("show"));
        assert!(menu.contains("more"));
        assert!(menu.contains("about"));
        assert!(menu.is_submenu("more"));
    }

    #[test]
    fn radio_groups_are_exclusive_and_stop_at_separators() {
        let mut menu = sample();
        menu.dispatch("setItemChecked", &json!({ "id": "large", "checked": true }))
            .unwrap()
            .unwrap();
        assert!(checked(&menu, "large"));
        assert!(!checked(&menu, "small"));
        assert!(checked(&menu, "light"), "a separator ends the group");

        assert!(menu.click("small"));
        assert!(checked(&menu, "small"));
        assert!(!checked(&menu, "large"));
        assert!(menu.on_activated("show"));
        assert!(!menu.on_activated("unknown"));
    }

    #[test]
    fn item_mutations_validate_arguments() {
        let mut menu = sample();
        let missing = menu
            .dispatch("setItemEnabled", &json!({ "id": "show" }))
            .unwrap()
            .unwrap_err();
        assert!(missing.contains("missing 'enabled'"), "{missing}");
        let unknown = menu
            .dispatch("setItemLabel", &json!({ "id": "nope", "label": "X" }))
            .unwrap()
            .unwrap_err();
        assert!(unknown.contains("'nope' not found"), "{unknown}");
        let not_check = menu
            .dispatch(
                "update",
                &json!({ "id": "show", "partial": { "checked": true } }),
            )
            .unwrap()
            .unwrap_err();
        assert!(not_check.contains("not a checkbox/radio"), "{not_check}");
        let bad_accel = menu
            .dispatch(
                "setItemAccelerator",
                &json!({ "id": "show", "accelerator": "Ctrl+Nope+" }),
            )
            .unwrap()
            .unwrap_err();
        assert!(bad_accel.contains("invalid accelerator"), "{bad_accel}");
        assert!(menu.dispatch("setIcon", &json!({})).is_none());
        assert!(
            menu.dispatch("remove", &json!({ "id": "show" })).is_none(),
            "tray.remove removes the icon, not an item"
        );
    }

    #[test]
    fn visibility_icon_append_and_remove() {
        let mut menu = sample();
        menu.dispatch("setItemVisible", &json!({ "id": "show", "visible": false }))
            .unwrap()
            .unwrap();
        menu.dispatch("setItemVisible", &json!({ "id": "show", "visible": true }))
            .unwrap()
            .unwrap();
        menu.dispatch("setItemIcon", &json!({ "id": "show", "nativeIcon": "add" }))
            .unwrap()
            .unwrap();
        assert!(menu.is_icon_item("show"));
        assert_eq!(menu.accelerator("show"), Some("CmdOrCtrl+S"));

        menu.dispatch(
            "append",
            &json!({ "parent": "more", "index": 0, "item": { "id": "help", "label": "Help" } }),
        )
        .unwrap()
        .unwrap();
        assert!(menu.contains("help"));
        let duplicate = menu
            .dispatch(
                "append",
                &json!({ "item": { "id": "help", "label": "Help" } }),
            )
            .unwrap()
            .unwrap_err();
        assert!(duplicate.contains("already exists"), "{duplicate}");
        let not_submenu = menu
            .dispatch(
                "append",
                &json!({ "parent": "show", "item": { "id": "x", "label": "X" } }),
            )
            .unwrap()
            .unwrap_err();
        assert!(not_submenu.contains("not a submenu"), "{not_submenu}");

        menu.dispatch("removeItem", &json!({ "id": "more" }))
            .unwrap()
            .unwrap();
        assert!(!menu.contains("more"));
        assert!(!menu.contains("help"), "descendants go with their submenu");
        assert!(!menu.contains("about"));
    }
}
