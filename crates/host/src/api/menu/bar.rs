//! Application menu-bar construction, registry-backed mutations, and OS initialization.

use std::{cell::RefCell, collections::HashMap};

use serde_json::Value;

use super::builder::{fill_with_registry, RegContainer, RegEntry, RegItem};
use tynd_protocol::runtime::MenuItemDef;

use crate::ipc::error::IpcError;
pub(crate) fn build_bar(items: &[MenuItemDef]) -> Result<muda::Menu, String> {
    let menu = muda::Menu::new();
    bar_registry_clear();
    // Non-submenu top-level entries without a label are silently skipped.
    for item in items {
        let MenuItemDef::Submenu(sub_def) = item else {
            continue;
        };
        let label = sub_def.label.as_deref().unwrap_or("");
        let enabled = sub_def.enabled.unwrap_or(true);
        let id_value = sub_def.id.as_deref().unwrap_or(label);
        let sub = if sub_def.id.is_some() {
            muda::Submenu::with_id(muda::MenuId::new(id_value), label, enabled)
        } else {
            muda::Submenu::new(label, enabled)
        };
        if let Some(id) = sub_def.id.as_deref() {
            bar_registry_insert(
                id.to_string(),
                BarEntry {
                    item: RegItem::Submenu(sub.clone()),
                    parent: RegContainer::Root(menu.clone()),
                    position: menu.items().len(),
                    visible: true,
                },
            );
        }
        let container = RegContainer::Sub(sub.clone());
        fill_with_registry(&container, &sub_def.items, &mut |id, e| {
            bar_registry_insert(id, bar_entry_from_reg(e));
        })?;
        menu.append(&sub).map_err(|e| e.to_string())?;
    }
    Ok(menu)
}

/// Per-id metadata for bar items. Mirrors `tray::Entry` (same registry
/// pattern) — `position` is the build-time index inside the parent so
/// `setItemVisible(id, true)` can reinsert at the original slot.
struct BarEntry {
    item: RegItem,
    parent: RegContainer,
    position: usize,
    visible: bool,
}

thread_local! {
    /// Main-thread-only registry of menu-bar items, keyed by the user-supplied
    /// `id`. The bar lives on the event-loop thread, the same as tray, so a
    /// thread-local is enough — and keeps the `!Send` muda handles from
    /// bleeding into global statics.
    static BAR_REGISTRY: RefCell<HashMap<String, BarEntry>> = RefCell::new(HashMap::new());
}

fn bar_registry_clear() {
    BAR_REGISTRY.with(|r| r.borrow_mut().clear());
}

fn bar_registry_insert(id: String, entry: BarEntry) {
    BAR_REGISTRY.with(|r| {
        r.borrow_mut().insert(id, entry);
    });
}

fn bar_entry_from_reg(e: RegEntry) -> BarEntry {
    BarEntry {
        item: e.item,
        parent: e.parent,
        position: e.position,
        visible: true,
    }
}

fn with_bar_entry<F, T>(id: &str, f: F) -> Result<T, String>
where
    F: FnOnce(&mut BarEntry) -> Result<T, String>,
{
    BAR_REGISTRY.with(|r| {
        let mut map = r.borrow_mut();
        let entry = map
            .get_mut(id)
            .ok_or_else(|| format!("menu: item id '{id}' not found"))?;
        f(entry)
    })
}

impl RegItem {
    fn as_is_menu_item(&self) -> &dyn muda::IsMenuItem {
        match self {
            RegItem::MenuItem(i) => i,
            RegItem::Submenu(i) => i,
            RegItem::Check(i) => i,
            RegItem::Predefined(i) => i,
        }
    }
}

fn bar_set_enabled(item: &RegItem, enabled: bool) {
    match item {
        RegItem::MenuItem(i) => i.set_enabled(enabled),
        RegItem::Submenu(i) => i.set_enabled(enabled),
        RegItem::Check(i) => i.set_enabled(enabled),
        // PredefinedMenuItem is OS-managed (e.g. quit, copy) — no setter.
        RegItem::Predefined(_) => {},
    }
}

fn bar_set_label(item: &RegItem, label: &str) {
    match item {
        RegItem::MenuItem(i) => i.set_text(label),
        RegItem::Submenu(i) => i.set_text(label),
        RegItem::Check(i) => i.set_text(label),
        RegItem::Predefined(i) => i.set_text(label),
    }
}

/// Runtime menu-bar mutations. Must run on the main thread — wired through
/// `UserEvent::MenuCmd` so muda's `!Send` handles never leave the event-loop
/// thread. Returns `Ok(Null)` on every successful mutation; the actual
/// state lives in the registry + muda items.
pub(crate) fn dispatch(method: &str, args: &Value) -> Result<Value, String> {
    match method {
        "update" => {
            let id = require_id(args, "update")?;
            apply_partial(&id, args)?;
            Ok(Value::Null)
        },
        "setItemEnabled" => {
            let id = require_id(args, "setItemEnabled")?;
            let enabled = args
                .get("enabled")
                .and_then(Value::as_bool)
                .ok_or_else(|| "menu.setItemEnabled: missing 'enabled'".to_string())?;
            with_bar_entry(&id, |entry| {
                bar_set_enabled(&entry.item, enabled);
                Ok(())
            })?;
            Ok(Value::Null)
        },
        "setItemLabel" => {
            let id = require_id(args, "setItemLabel")?;
            let label = args
                .get("label")
                .and_then(Value::as_str)
                .ok_or_else(|| "menu.setItemLabel: missing 'label'".to_string())?
                .to_string();
            with_bar_entry(&id, |entry| {
                bar_set_label(&entry.item, &label);
                Ok(())
            })?;
            Ok(Value::Null)
        },
        "setItemChecked" => {
            let id = require_id(args, "setItemChecked")?;
            let checked = args
                .get("checked")
                .and_then(Value::as_bool)
                .ok_or_else(|| "menu.setItemChecked: missing 'checked'".to_string())?;
            with_bar_entry(&id, |entry| match &entry.item {
                RegItem::Check(c) => {
                    c.set_checked(checked);
                    Ok(())
                },
                _ => Err(format!(
                    "menu.setItemChecked: item '{id}' is not a checkbox/radio"
                )),
            })?;
            Ok(Value::Null)
        },
        "setItemVisible" => {
            let id = require_id(args, "setItemVisible")?;
            let visible = args
                .get("visible")
                .and_then(Value::as_bool)
                .ok_or_else(|| "menu.setItemVisible: missing 'visible'".to_string())?;
            set_visible(&id, visible)?;
            Ok(Value::Null)
        },
        _ => Err(IpcError::unknown_method("menu", method).into_string()),
    }
}

fn require_id(args: &Value, op: &str) -> Result<String, String> {
    args.get("id")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| format!("menu.{op}: missing 'id'"))
}

/// `update(id, partial)` core. Walks the partial object once and applies
/// every present field — label, enabled, checked, visible — so callers
/// can batch mutations in a single round trip. Each field falls back to
/// the same per-field error path as the convenience setters so the user
/// gets the precise reason ("not a checkbox/radio") rather than a generic
/// "update failed".
fn apply_partial(id: &str, args: &Value) -> Result<(), String> {
    // The entire `args` shape is `{ id, partial: { label?, enabled?, checked?, visible? } }`.
    let partial = args
        .get("partial")
        .filter(|partial| partial.is_object())
        .ok_or_else(|| "menu.update: missing 'partial' object".to_string())?;

    let checked = partial.get("checked").and_then(Value::as_bool);
    if checked.is_some() {
        with_bar_entry(id, |entry| match &entry.item {
            RegItem::Check(_) => Ok(()),
            _ => Err(format!(
                "menu.update: item '{id}' is not a checkbox/radio (cannot set 'checked')"
            )),
        })?;
    }

    // Visibility is the only fallible native mutation. Apply it before the
    // infallible setters so an error cannot leave a partially-updated item.
    if let Some(visible) = partial.get("visible").and_then(Value::as_bool) {
        set_visible(id, visible)?;
    }
    if let Some(label) = partial.get("label").and_then(Value::as_str) {
        with_bar_entry(id, |entry| {
            bar_set_label(&entry.item, label);
            Ok(())
        })?;
    }
    if let Some(enabled) = partial.get("enabled").and_then(Value::as_bool) {
        with_bar_entry(id, |entry| {
            bar_set_enabled(&entry.item, enabled);
            Ok(())
        })?;
    }
    if let Some(checked) = checked {
        with_bar_entry(id, |entry| {
            let RegItem::Check(item) = &entry.item else {
                unreachable!("checked item validated before mutations");
            };
            item.set_checked(checked);
            Ok(())
        })?;
    }
    Ok(())
}

fn set_visible(id: &str, visible: bool) -> Result<(), String> {
    BAR_REGISTRY.with(|registry| {
        let mut entries = registry.borrow_mut();
        let (position, parent, already_visible) = entries
            .get(id)
            .map(|entry| (entry.position, entry.parent.clone(), entry.visible))
            .ok_or_else(|| format!("menu: item id '{id}' not found"))?;
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
            .get_mut(id)
            .expect("entry existence checked before sibling scan");
        if visible {
            let target = position.saturating_sub(hidden_before).min(parent.len());
            parent
                .insert(entry.item.as_is_menu_item(), target)
                .map_err(|error| format!("menu.setItemVisible: {error}"))?;
        } else {
            parent
                .remove(entry.item.as_is_menu_item())
                .map_err(|error| format!("menu.setItemVisible: {error}"))?;
        }
        entry.visible = visible;
        Ok(())
    })
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
