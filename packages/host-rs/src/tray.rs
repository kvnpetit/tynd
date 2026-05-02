use std::{cell::RefCell, collections::HashMap};

use serde_json::Value;

use crate::{menu, os, runtime::TrayConfig};

/// Per-item handle stored in the tray menu registry. Wraps the muda items
/// we may want to mutate at runtime — `setItemEnabled` works on the first
/// three variants, `setItemChecked` only on `Check`, `setItemLabel` on the
/// first three (predefined items have a `set_text` too but their text is
/// usually OS-managed). `Predefined` is tracked so visibility (remove/insert)
/// works for native roles like "quit" / "copy".
#[derive(Clone)]
enum ItemRef {
    MenuItem(muda::MenuItem),
    Submenu(muda::Submenu),
    Check(muda::CheckMenuItem),
    Predefined(muda::PredefinedMenuItem),
}

impl ItemRef {
    fn as_is_menu_item(&self) -> &dyn muda::IsMenuItem {
        match self {
            ItemRef::MenuItem(i) => i,
            ItemRef::Submenu(i) => i,
            ItemRef::Check(i) => i,
            ItemRef::Predefined(i) => i,
        }
    }
}

/// Per-id metadata cached so we can mutate items without rebuilding the
/// whole menu. `position` is the item's index inside its parent at build
/// time — used as the insertion offset when toggling visibility back on
/// (clamped to the parent's current length to handle siblings being hidden
/// in arbitrary order).
struct Entry {
    item: ItemRef,
    parent: menu::RegContainer,
    position: usize,
    visible: bool,
}

thread_local! {
    /// Main-thread-only registry of tray menu items, keyed by user-supplied
    /// `id`. `tray::dispatch` runs on the event loop thread so a thread-local
    /// is sufficient — and avoids the `!Send` muda handles bleeding into
    /// global statics.
    static REGISTRY: RefCell<HashMap<String, Entry>> = RefCell::new(HashMap::new());
}

fn registry_clear() {
    REGISTRY.with(|r| r.borrow_mut().clear());
}

fn registry_insert(id: String, entry: Entry) {
    REGISTRY.with(|r| {
        r.borrow_mut().insert(id, entry);
    });
}

fn with_entry<F, T>(id: &str, f: F) -> Result<T, String>
where
    F: FnOnce(&mut Entry) -> Result<T, String>,
{
    REGISTRY.with(|r| {
        let mut map = r.borrow_mut();
        let entry = map
            .get_mut(id)
            .ok_or_else(|| format!("tray: item id '{id}' not found"))?;
        f(entry)
    })
}

pub(crate) fn build(cfg: &TrayConfig) -> Result<tray_icon::TrayIcon, String> {
    let icon = os::icon::load_tray(&cfg.icon)?;

    let tray_menu = muda::Menu::new();
    registry_clear();
    if let Some(items) = &cfg.menu {
        let container = menu::RegContainer::Root(tray_menu.clone());
        menu::fill_with_registry(&container, items, &mut |id, entry| {
            registry_insert(id, entry_to_tray(entry));
        })?;
    }

    let mut builder = tray_icon::TrayIconBuilder::new()
        .with_icon(icon)
        .with_menu(Box::new(tray_menu));

    if let Some(tt) = &cfg.tooltip {
        builder = builder.with_tooltip(tt);
    }

    builder.build().map_err(|e| e.to_string())
}

/// Bridge between `menu::RegEntry` (the cross-module record produced by the
/// shared menu builder) and the tray-local `Entry` shape. Keeps `menu.rs`
/// generic over the storage strategy — the menu bar can later reuse the
/// same builder with its own registry without dragging tray-specific
/// fields (visibility flag) into the menu module.
fn entry_to_tray(e: menu::RegEntry) -> Entry {
    let item = match e.item {
        menu::RegItem::MenuItem(i) => ItemRef::MenuItem(i),
        menu::RegItem::Submenu(s) => ItemRef::Submenu(s),
        menu::RegItem::Check(c) => ItemRef::Check(c),
        menu::RegItem::Predefined(p) => ItemRef::Predefined(p),
    };
    Entry {
        item,
        parent: e.parent,
        position: e.position,
        visible: true,
    }
}

/// Runtime tray mutations — must run on the main thread. Called via the
/// `UserEvent::TrayCmd` path so Windows / macOS invariants are preserved.
pub(crate) fn dispatch(
    tray: &tray_icon::TrayIcon,
    method: &str,
    args: &Value,
) -> Result<Value, String> {
    match method {
        "setIcon" => {
            let path = args
                .get("path")
                .and_then(Value::as_str)
                .ok_or_else(|| "tray.setIcon: missing 'path'".to_string())?;
            let icon = os::icon::load_tray(path)?;
            tray.set_icon(Some(icon)).map_err(|e| e.to_string())?;
            Ok(Value::Null)
        },
        "setTooltip" => {
            let text = args.get("text").and_then(Value::as_str);
            tray.set_tooltip(text).map_err(|e| e.to_string())?;
            Ok(Value::Null)
        },
        "setTitle" => {
            // macOS only — sets text next to the icon in the menu bar.
            let text = args.get("text").and_then(Value::as_str);
            tray.set_title(text);
            Ok(Value::Null)
        },
        "setVisible" => {
            let visible = args.get("visible").and_then(Value::as_bool).unwrap_or(true);
            tray.set_visible(visible).map_err(|e| e.to_string())?;
            Ok(Value::Null)
        },
        "setMenu" => {
            let items = args.get("items").and_then(Value::as_array).cloned();
            let new_menu = muda::Menu::new();
            registry_clear();
            if let Some(items) = items {
                // Re-use the TS menu schema — items carry the same shape as
                // tynd.config.ts::tray.menu, so we delegate to the shared
                // builder instead of duplicating the walk here.
                let parsed: Vec<crate::runtime::MenuItemDef> =
                    serde_json::from_value(Value::Array(items))
                        .map_err(|e| format!("tray.setMenu: invalid items — {e}"))?;
                let container = menu::RegContainer::Root(new_menu.clone());
                menu::fill_with_registry(&container, &parsed, &mut |id, entry| {
                    registry_insert(id, entry_to_tray(entry));
                })?;
            }
            tray.set_menu(Some(Box::new(new_menu)));
            Ok(Value::Null)
        },
        "setIconAsTemplate" => {
            // macOS: template icons automatically adapt to light / dark menu
            // bar without requiring a separate image. No-op on Win / Linux.
            let is_template = args
                .get("template")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            tray.set_icon_as_template(is_template);
            Ok(Value::Null)
        },
        "setItemEnabled" => {
            let id = require_id(args, "setItemEnabled")?;
            let enabled = args
                .get("enabled")
                .and_then(Value::as_bool)
                .ok_or_else(|| "tray.setItemEnabled: missing 'enabled'".to_string())?;
            with_entry(&id, |entry| {
                set_enabled(&entry.item, enabled);
                Ok(())
            })?;
            Ok(Value::Null)
        },
        "setItemLabel" => {
            let id = require_id(args, "setItemLabel")?;
            let label = args
                .get("label")
                .and_then(Value::as_str)
                .ok_or_else(|| "tray.setItemLabel: missing 'label'".to_string())?
                .to_string();
            with_entry(&id, |entry| {
                set_label(&entry.item, &label).map_err(|e| format!("tray.setItemLabel: {e}"))?;
                Ok(())
            })?;
            Ok(Value::Null)
        },
        "setItemChecked" => {
            let id = require_id(args, "setItemChecked")?;
            let checked = args
                .get("checked")
                .and_then(Value::as_bool)
                .ok_or_else(|| "tray.setItemChecked: missing 'checked'".to_string())?;
            with_entry(&id, |entry| match &entry.item {
                ItemRef::Check(c) => {
                    c.set_checked(checked);
                    Ok(())
                },
                _ => Err(format!(
                    "tray.setItemChecked: item '{id}' is not a checkbox/radio"
                )),
            })?;
            Ok(Value::Null)
        },
        "setItemVisible" => {
            let id = require_id(args, "setItemVisible")?;
            let visible = args
                .get("visible")
                .and_then(Value::as_bool)
                .ok_or_else(|| "tray.setItemVisible: missing 'visible'".to_string())?;
            with_entry(&id, |entry| {
                if entry.visible == visible {
                    return Ok(());
                }
                if visible {
                    // Re-insertion at the original position; clamp because
                    // sibling hides may have shrunk the parent in between.
                    let target = entry.position.min(entry.parent.len());
                    entry
                        .parent
                        .insert(entry.item.as_is_menu_item(), target)
                        .map_err(|e| format!("tray.setItemVisible: {e}"))?;
                } else {
                    entry
                        .parent
                        .remove(entry.item.as_is_menu_item())
                        .map_err(|e| format!("tray.setItemVisible: {e}"))?;
                }
                entry.visible = visible;
                Ok(())
            })?;
            Ok(Value::Null)
        },
        _ => Err(format!("tray.{method}: unknown method")),
    }
}

fn require_id(args: &Value, op: &str) -> Result<String, String> {
    args.get("id")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| format!("tray.{op}: missing 'id'"))
}

fn set_enabled(item: &ItemRef, enabled: bool) {
    match item {
        ItemRef::MenuItem(i) => i.set_enabled(enabled),
        ItemRef::Submenu(i) => i.set_enabled(enabled),
        ItemRef::Check(i) => i.set_enabled(enabled),
        // PredefinedMenuItem has no set_enabled — OS-managed roles like
        // "quit" / "copy" stay always enabled. Silently ignored so callers
        // don't have to type-check before toggling.
        ItemRef::Predefined(_) => {},
    }
}

fn set_label(item: &ItemRef, label: &str) -> Result<(), String> {
    match item {
        ItemRef::MenuItem(i) => {
            i.set_text(label);
            Ok(())
        },
        ItemRef::Submenu(i) => {
            i.set_text(label);
            Ok(())
        },
        ItemRef::Check(i) => {
            i.set_text(label);
            Ok(())
        },
        ItemRef::Predefined(i) => {
            i.set_text(label);
            Ok(())
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::MenuItemDef;
    use serde_json::json;

    fn item(id: &str, label: &str) -> MenuItemDef {
        MenuItemDef {
            id: Some(id.to_string()),
            label: Some(label.to_string()),
            ..Default::default()
        }
    }

    fn checkbox(id: &str, label: &str, checked: bool) -> MenuItemDef {
        MenuItemDef {
            kind: Some("checkbox".to_string()),
            id: Some(id.to_string()),
            label: Some(label.to_string()),
            checked: Some(checked),
            ..Default::default()
        }
    }

    /// Build a populated menu + registry without going through the
    /// `tray_icon::TrayIcon` builder (which needs an event loop on macOS /
    /// Linux). The mutation paths only touch muda + the registry, so this
    /// is enough to exercise them headlessly on every platform.
    fn build_menu(items: &[MenuItemDef]) -> muda::Menu {
        let m = muda::Menu::new();
        registry_clear();
        let container = menu::RegContainer::Root(m.clone());
        menu::fill_with_registry(&container, items, &mut |id, entry| {
            registry_insert(id, entry_to_tray(entry));
        })
        .expect("fill_with_registry");
        m
    }

    fn fake_tray_dispatch_label(id: &str, label: &str) -> Result<(), String> {
        // Mirror the real arms but skip the TrayIcon (only `setItem*` paths
        // use the registry, never the tray handle). The intent is to keep
        // the tests OS-agnostic — `tray_icon::TrayIcon` requires GTK on Linux
        // and an `NSApplication` on macOS, neither of which exist under
        // `cargo test`.
        with_entry(id, |entry| {
            set_label(&entry.item, label).map_err(|e| format!("tray.setItemLabel: {e}"))
        })
    }

    #[test]
    fn registry_populates_per_id() {
        build_menu(&[item("one", "One"), item("two", "Two"), item("three", "Three")]);
        REGISTRY.with(|r| {
            let map = r.borrow();
            assert!(map.contains_key("one"));
            assert!(map.contains_key("two"));
            assert!(map.contains_key("three"));
            assert_eq!(map.len(), 3);
        });
    }

    #[test]
    fn unknown_id_returns_clear_error() {
        build_menu(&[item("only", "Only")]);
        let err = with_entry("ghost", |_| Ok::<_, String>(())).unwrap_err();
        assert!(err.contains("'ghost' not found"), "got: {err}");
    }

    #[test]
    fn set_enabled_flips_muda_state() {
        build_menu(&[item("toggle", "Toggle")]);
        // Default is enabled.
        REGISTRY.with(|r| {
            let map = r.borrow();
            let entry = map.get("toggle").unwrap();
            if let ItemRef::MenuItem(mi) = &entry.item {
                assert!(mi.is_enabled());
            } else {
                panic!("expected MenuItem");
            }
        });
        with_entry("toggle", |entry| {
            set_enabled(&entry.item, false);
            Ok::<_, String>(())
        })
        .unwrap();
        REGISTRY.with(|r| {
            let map = r.borrow();
            let entry = map.get("toggle").unwrap();
            if let ItemRef::MenuItem(mi) = &entry.item {
                assert!(!mi.is_enabled());
            }
        });
    }

    #[test]
    fn set_label_updates_text() {
        build_menu(&[item("first", "First")]);
        fake_tray_dispatch_label("first", "Renamed").unwrap();
        REGISTRY.with(|r| {
            let map = r.borrow();
            let entry = map.get("first").unwrap();
            if let ItemRef::MenuItem(mi) = &entry.item {
                // Some platforms (notably Windows) prepend an `&` accelerator hint;
                // assert via `contains` rather than equality so the test stays
                // OS-portable. The point is that the new label took effect.
                assert!(mi.text().contains("Renamed"), "got: {}", mi.text());
            }
        });
    }

    #[test]
    fn set_checked_only_on_checkbox() {
        build_menu(&[checkbox("muted", "Mute", false), item("plain", "Plain")]);

        // Checkbox toggles cleanly.
        with_entry("muted", |entry| match &entry.item {
            ItemRef::Check(c) => {
                c.set_checked(true);
                Ok::<_, String>(())
            },
            _ => Err("not a check".into()),
        })
        .unwrap();
        REGISTRY.with(|r| {
            let map = r.borrow();
            let entry = map.get("muted").unwrap();
            if let ItemRef::Check(c) = &entry.item {
                assert!(c.is_checked());
            } else {
                panic!("expected Check");
            }
        });

        // Plain MenuItem rejects setItemChecked.
        let err = with_entry("plain", |entry| match &entry.item {
            ItemRef::Check(_) => Ok(()),
            _ => Err("tray.setItemChecked: item 'plain' is not a checkbox/radio".to_string()),
        })
        .unwrap_err();
        assert!(err.contains("not a checkbox"), "got: {err}");
    }

    #[test]
    fn visibility_toggle_removes_and_reinserts() {
        let m = build_menu(&[
            item("a", "A"),
            item("b", "B"),
            item("c", "C"),
        ]);
        assert_eq!(m.items().len(), 3);

        // Hide the middle item.
        with_entry("b", |entry| {
            entry
                .parent
                .remove(entry.item.as_is_menu_item())
                .map_err(|e| e.to_string())?;
            entry.visible = false;
            Ok::<_, String>(())
        })
        .unwrap();
        assert_eq!(m.items().len(), 2);

        // Show it back at the original position.
        with_entry("b", |entry| {
            let target = entry.position.min(entry.parent.len());
            entry
                .parent
                .insert(entry.item.as_is_menu_item(), target)
                .map_err(|e| e.to_string())?;
            entry.visible = true;
            Ok::<_, String>(())
        })
        .unwrap();
        assert_eq!(m.items().len(), 3);
    }

    #[test]
    fn require_id_validates_args() {
        let err = require_id(&json!({}), "setItemEnabled").unwrap_err();
        assert!(err.contains("missing 'id'"));
        let id = require_id(&json!({ "id": "ok" }), "setItemEnabled").unwrap();
        assert_eq!(id, "ok");
    }

    #[test]
    fn submenu_children_are_registered_too() {
        let items = vec![MenuItemDef {
            kind: Some("submenu".to_string()),
            id: Some("file".to_string()),
            label: Some("File".to_string()),
            items: Some(vec![item("open", "Open"), item("save", "Save")]),
            ..Default::default()
        }];
        build_menu(&items);
        REGISTRY.with(|r| {
            let map = r.borrow();
            assert!(map.contains_key("file"));
            assert!(map.contains_key("open"));
            assert!(map.contains_key("save"));
        });
    }
}
