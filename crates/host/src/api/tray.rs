use std::{cell::RefCell, collections::HashMap};

use serde_json::Value;

use tynd_protocol::runtime::TrayConfig;

use crate::{api::menu, ipc::error::IpcError};

/// Tray menu item handle. `Predefined` is tracked so visibility toggles work
/// on native roles like "quit" / "copy".
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

/// Per-id cached metadata so we can mutate items without rebuilding the menu.
/// `position` is the build-time index, used as the re-insert offset when a
/// hidden item is shown (clamped to the parent's current length).
struct Entry {
    item: ItemRef,
    parent: menu::builder::RegContainer,
    position: usize,
    visible: bool,
}

thread_local! {
    /// Main-thread tray menu registry. Thread-local keeps `!Send` muda
    /// handles out of globals.
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

/// The live tray handle held by the event loop.
pub(crate) type SystemTray = tray_icon::TrayIcon;

/// Error returned when a renderer calls `tray.*` before a tray was configured.
pub(crate) fn unavailable_error() -> String {
    "tray: no tray configured — set `tray` in tynd.config.ts first".into()
}

pub(crate) fn build(cfg: &TrayConfig) -> Result<tray_icon::TrayIcon, String> {
    let icon = crate::platform::icon::load_tray(&cfg.icon)?;

    let tray_menu = muda::Menu::new();
    registry_clear();
    if let Some(items) = &cfg.menu {
        let container = menu::builder::RegContainer::Root(tray_menu.clone());
        menu::builder::fill_with_registry(&container, items, &mut |id, entry| {
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

/// Convert a `menu::builder::RegEntry` into the tray-local `Entry` (with the
/// tray-specific visibility flag).
fn entry_to_tray(e: menu::builder::RegEntry) -> Entry {
    let item = match e.item {
        menu::builder::RegItem::MenuItem(i) => ItemRef::MenuItem(i),
        menu::builder::RegItem::Submenu(s) => ItemRef::Submenu(s),
        menu::builder::RegItem::Check(c) => ItemRef::Check(c),
        menu::builder::RegItem::Predefined(p) => ItemRef::Predefined(p),
    };
    Entry {
        item,
        parent: e.parent,
        position: e.position,
        visible: true,
    }
}

/// Runtime tray mutations — main-thread only. Routed via `UserEvent::TrayCmd`.
pub(crate) fn dispatch(
    tray: &tray_icon::TrayIcon,
    method: &str,
    args: &Value,
) -> Result<Value, String> {
    match method {
        "setIcon" => dispatch_set_icon(tray, args),
        "setTooltip" => dispatch_set_tooltip(tray, args),
        "setTitle" => dispatch_set_title(tray, args),
        "setVisible" => dispatch_set_visible(tray, args),
        "setMenu" => dispatch_set_menu(tray, args),
        "setIconAsTemplate" => dispatch_set_icon_as_template(tray, args),
        "setItemEnabled" => dispatch_set_item_enabled(args),
        "setItemLabel" => dispatch_set_item_label(args),
        "setItemChecked" => dispatch_set_item_checked(args),
        "setItemVisible" => dispatch_set_item_visible(args),
        _ => Err(IpcError::unknown_method("tray", method).into_string()),
    }
}

fn dispatch_set_icon(tray: &tray_icon::TrayIcon, args: &Value) -> Result<Value, String> {
    let path = args
        .get("path")
        .and_then(Value::as_str)
        .ok_or_else(|| "tray.setIcon: missing 'path'".to_string())?;
    let icon = crate::platform::icon::load_tray(path)?;
    tray.set_icon(Some(icon))
        .map_err(|error| error.to_string())?;
    Ok(Value::Null)
}

fn dispatch_set_tooltip(tray: &tray_icon::TrayIcon, args: &Value) -> Result<Value, String> {
    let text = args.get("text").and_then(Value::as_str);
    tray.set_tooltip(text).map_err(|error| error.to_string())?;
    Ok(Value::Null)
}

fn dispatch_set_title(tray: &tray_icon::TrayIcon, args: &Value) -> Result<Value, String> {
    // macOS only — sets text next to the icon in the menu bar.
    let text = args.get("text").and_then(Value::as_str);
    tray.set_title(text);
    Ok(Value::Null)
}

fn dispatch_set_visible(tray: &tray_icon::TrayIcon, args: &Value) -> Result<Value, String> {
    let visible = args.get("visible").and_then(Value::as_bool).unwrap_or(true);
    tray.set_visible(visible)
        .map_err(|error| error.to_string())?;
    Ok(Value::Null)
}

fn dispatch_set_menu(tray: &tray_icon::TrayIcon, args: &Value) -> Result<Value, String> {
    let items = args.get("items").and_then(Value::as_array).cloned();
    let new_menu = muda::Menu::new();
    registry_clear();
    if let Some(items) = items {
        let parsed: Vec<tynd_protocol::runtime::MenuItemDef> =
            serde_json::from_value(Value::Array(items))
                .map_err(|error| format!("tray.setMenu: invalid items — {error}"))?;
        let container = menu::builder::RegContainer::Root(new_menu.clone());
        menu::builder::fill_with_registry(&container, &parsed, &mut |id, entry| {
            registry_insert(id, entry_to_tray(entry));
        })?;
    }
    tray.set_menu(Some(Box::new(new_menu)));
    Ok(Value::Null)
}

// tray-icon 0.26 deprecates `set_icon_as_template` in favour of the macOS-only
// `set_icon_templated(Option<Icon>)`, which needs the icon re-supplied. The
// deprecated call keeps the existing "toggle the current icon" contract on all
// platforms (no-op off macOS), so it stays until the API can be re-shaped.
#[allow(deprecated)]
fn dispatch_set_icon_as_template(
    tray: &tray_icon::TrayIcon,
    args: &Value,
) -> Result<Value, String> {
    // macOS only — template icons adapt to light/dark. No-op elsewhere.
    let is_template = args
        .get("template")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    tray.set_icon_as_template(is_template);
    Ok(Value::Null)
}

fn dispatch_set_item_enabled(args: &Value) -> Result<Value, String> {
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
}

fn dispatch_set_item_label(args: &Value) -> Result<Value, String> {
    let id = require_id(args, "setItemLabel")?;
    let label = args
        .get("label")
        .and_then(Value::as_str)
        .ok_or_else(|| "tray.setItemLabel: missing 'label'".to_string())?
        .to_string();
    with_entry(&id, |entry| {
        set_label(&entry.item, &label).map_err(|error| format!("tray.setItemLabel: {error}"))?;
        Ok(())
    })?;
    Ok(Value::Null)
}

fn dispatch_set_item_checked(args: &Value) -> Result<Value, String> {
    let id = require_id(args, "setItemChecked")?;
    let checked = args
        .get("checked")
        .and_then(Value::as_bool)
        .ok_or_else(|| "tray.setItemChecked: missing 'checked'".to_string())?;
    with_entry(&id, |entry| match &entry.item {
        ItemRef::Check(item) => {
            item.set_checked(checked);
            Ok(())
        },
        _ => Err(format!(
            "tray.setItemChecked: item '{id}' is not a checkbox/radio"
        )),
    })?;
    Ok(Value::Null)
}

fn dispatch_set_item_visible(args: &Value) -> Result<Value, String> {
    let id = require_id(args, "setItemVisible")?;
    let visible = args
        .get("visible")
        .and_then(Value::as_bool)
        .ok_or_else(|| "tray.setItemVisible: missing 'visible'".to_string())?;
    REGISTRY.with(|registry| -> Result<(), String> {
        let mut entries = registry.borrow_mut();
        let (position, parent, already_visible) = entries
            .get(&id)
            .map(|entry| (entry.position, entry.parent.clone(), entry.visible))
            .ok_or_else(|| format!("tray: item id '{id}' not found"))?;
        if already_visible == visible {
            return Ok(());
        }

        let hidden_before = entries
            .values()
            .filter(|entry| {
                !entry.visible && entry.position < position && entry.parent.same_as(&parent)
            })
            .count();
        let entry = entries
            .get_mut(&id)
            .expect("entry existence checked before sibling scan");
        if visible {
            let target = position.saturating_sub(hidden_before).min(parent.len());
            parent
                .insert(entry.item.as_is_menu_item(), target)
                .map_err(|error| format!("tray.setItemVisible: {error}"))?;
        } else {
            parent
                .remove(entry.item.as_is_menu_item())
                .map_err(|error| format!("tray.setItemVisible: {error}"))?;
        }
        entry.visible = visible;
        Ok(())
    })?;
    Ok(Value::Null)
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

    #[test]
    fn unavailable_error_asks_for_tray_config() {
        assert!(unavailable_error().contains("no tray configured"));
    }
}
