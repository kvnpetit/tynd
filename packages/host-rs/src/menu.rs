use std::{cell::RefCell, collections::HashMap, str::FromStr as _};

use muda::{accelerator::Accelerator, ContextMenu as _};
use serde_json::Value;

use crate::runtime::MenuItemDef;

/// Concrete muda handles produced by the registering builder. Mirrors
/// `muda::MenuItemKind` but covers only the four variants the tray (and
/// future menu-bar) mutation API needs to look up by id. Kept in `menu`
/// so the wider crate can reuse the builder without re-deriving the
/// dispatch over muda's enum.
#[derive(Clone)]
pub(crate) enum RegItem {
    MenuItem(muda::MenuItem),
    Submenu(muda::Submenu),
    Check(muda::CheckMenuItem),
    Predefined(muda::PredefinedMenuItem),
}

/// Parent container the item was appended to. Doubles as the public
/// "build target" for [`fill_with_registry`]: the same enum drives the
/// initial append walk and the later `insert` / `remove` calls used by
/// `setItemVisible`. Cloneable because muda items are `Rc`-backed
/// internally — cloning is just a refcount bump.
#[derive(Clone)]
pub(crate) enum RegContainer {
    Root(muda::Menu),
    Sub(muda::Submenu),
}

impl RegContainer {
    pub(crate) fn append(&self, item: &dyn muda::IsMenuItem) -> muda::Result<()> {
        match self {
            RegContainer::Root(m) => m.append(item),
            RegContainer::Sub(s) => s.append(item),
        }
    }

    pub(crate) fn insert(&self, item: &dyn muda::IsMenuItem, position: usize) -> muda::Result<()> {
        match self {
            RegContainer::Root(m) => m.insert(item, position),
            RegContainer::Sub(s) => s.insert(item, position),
        }
    }

    pub(crate) fn remove(&self, item: &dyn muda::IsMenuItem) -> muda::Result<()> {
        match self {
            RegContainer::Root(m) => m.remove(item),
            RegContainer::Sub(s) => s.remove(item),
        }
    }

    pub(crate) fn len(&self) -> usize {
        match self {
            RegContainer::Root(m) => m.items().len(),
            RegContainer::Sub(s) => s.items().len(),
        }
    }
}

/// One entry produced per registered item: what it is, where it lives, and
/// at what index inside its parent at build time. The index is preserved
/// so visibility toggles can re-insert at (close to) the original slot.
pub(crate) struct RegEntry {
    pub item: RegItem,
    pub parent: RegContainer,
    pub position: usize,
}

fn parse_accel(item: &MenuItemDef) -> Option<Accelerator> {
    let s = item.accelerator.as_deref()?;
    match Accelerator::from_str(s) {
        Ok(a) => Some(a),
        Err(e) => {
            crate::tynd_log!("menu: bad accelerator {s:?}: {e}");
            None
        },
    }
}

pub(crate) fn build_bar(items: &[MenuItemDef]) -> Result<muda::Menu, String> {
    let menu = muda::Menu::new();
    bar_registry_clear();
    for item in items {
        let label = item.label.as_deref().unwrap_or("");
        let enabled = item.enabled.unwrap_or(true);
        let id_value = item.id.as_deref().unwrap_or(label);
        let sub = if item.id.is_some() {
            muda::Submenu::with_id(muda::MenuId::new(id_value), label, enabled)
        } else {
            muda::Submenu::new(label, enabled)
        };
        // Top-level submenu (the "File" / "Edit" entry on the bar). Track it
        // by id so `menu.update("file", { label: "Fichier" })` can rename
        // entire bar entries on the fly.
        if let Some(id) = item.id.as_deref() {
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
        // Recurse into the children — each addressable child lands in the
        // same registry so `menu.update("file.save", { ... })` works without
        // walking the muda tree.
        let container = RegContainer::Sub(sub.clone());
        fill_with_registry(&container, item.items.as_deref().unwrap_or(&[]), &mut |id, e| {
            bar_registry_insert(id, bar_entry_from_reg(e));
        })?;
        menu.append(&sub).map_err(|e| e.to_string())?;
    }
    Ok(menu)
}

/// Recursively fill a menu container using a caller-provided append closure.
pub(crate) fn fill(
    append: &dyn Fn(&dyn muda::IsMenuItem) -> muda::Result<()>,
    items: &[MenuItemDef],
) -> Result<(), String> {
    for item in items {
        match item.kind.as_deref() {
            Some("separator") => {
                append(&muda::PredefinedMenuItem::separator()).map_err(|e| e.to_string())?;
            },
            Some("submenu") => {
                let label = item.label.as_deref().unwrap_or("");
                let enabled = item.enabled.unwrap_or(true);
                let sub = muda::Submenu::new(label, enabled);
                fill(
                    &|i: &dyn muda::IsMenuItem| sub.append(i),
                    item.items.as_deref().unwrap_or(&[]),
                )?;
                append(&sub).map_err(|e| e.to_string())?;
            },
            Some("checkbox") => {
                if let Some(label) = &item.label {
                    let id = muda::MenuId::new(item.id.as_deref().unwrap_or(label));
                    let mi = muda::CheckMenuItem::with_id(
                        id,
                        label,
                        item.enabled.unwrap_or(true),
                        item.checked.unwrap_or(false),
                        parse_accel(item),
                    );
                    append(&mi).map_err(|e| e.to_string())?;
                }
            },
            Some("radio") => {
                // muda has no native radio group — approximate with CheckMenuItem.
                // Apps enforce single-selection by listening for clicks and
                // calling `menu.setChecked(id, bool)` to uncheck siblings.
                if let Some(label) = &item.label {
                    let id = muda::MenuId::new(item.id.as_deref().unwrap_or(label));
                    let mi = muda::CheckMenuItem::with_id(
                        id,
                        label,
                        item.enabled.unwrap_or(true),
                        item.checked.unwrap_or(false),
                        parse_accel(item),
                    );
                    append(&mi).map_err(|e| e.to_string())?;
                }
            },
            _ => {
                if let Some(role) = &item.role {
                    if let Some(pi) = role_to_predefined(role) {
                        append(&pi).map_err(|e| e.to_string())?;
                        continue;
                    }
                }
                if let Some(label) = &item.label {
                    let id = muda::MenuId::new(item.id.as_deref().unwrap_or(label));
                    let mi = muda::MenuItem::with_id(
                        id,
                        label,
                        item.enabled.unwrap_or(true),
                        parse_accel(item),
                    );
                    append(&mi).map_err(|e| e.to_string())?;
                }
            },
        }
    }
    Ok(())
}

/// Like [`fill`] but tracks every appended item in a caller-supplied
/// registry callback. Used by `tray.rs` to keep an `id -> handle` map
/// alongside the muda menu so subsequent `setItemEnabled` / `setItemLabel`
/// / `setItemChecked` / `setItemVisible` calls can locate the item without
/// rebuilding the whole menu.
///
/// Items without an `id` (separators, label-less roles) are still appended
/// — they're just not registered, which mirrors the behaviour of `fill`:
/// only addressable items participate in the dynamic API.
pub(crate) fn fill_with_registry<F>(
    container: &RegContainer,
    items: &[MenuItemDef],
    register: &mut F,
) -> Result<(), String>
where
    F: FnMut(String, RegEntry),
{
    let start_position = container.len();

    for (offset, item) in items.iter().enumerate() {
        let position = start_position + offset;
        match item.kind.as_deref() {
            Some("separator") => {
                let pi = muda::PredefinedMenuItem::separator();
                container.append(&pi).map_err(|e| e.to_string())?;
                if let Some(id) = item.id.as_deref() {
                    register(
                        id.to_string(),
                        RegEntry {
                            item: RegItem::Predefined(pi),
                            parent: container.clone(),
                            position,
                        },
                    );
                }
            },
            Some("submenu") => {
                let label = item.label.as_deref().unwrap_or("");
                let enabled = item.enabled.unwrap_or(true);
                let id_value = item.id.as_deref().unwrap_or(label);
                let sub = muda::Submenu::with_id(muda::MenuId::new(id_value), label, enabled);
                fill_with_registry(
                    &RegContainer::Sub(sub.clone()),
                    item.items.as_deref().unwrap_or(&[]),
                    register,
                )?;
                container.append(&sub).map_err(|e| e.to_string())?;
                if let Some(id) = item.id.as_deref() {
                    register(
                        id.to_string(),
                        RegEntry {
                            item: RegItem::Submenu(sub),
                            parent: container.clone(),
                            position,
                        },
                    );
                }
            },
            Some("checkbox") | Some("radio") => {
                if let Some(label) = &item.label {
                    let id_value = item.id.as_deref().unwrap_or(label);
                    let id = muda::MenuId::new(id_value);
                    let mi = muda::CheckMenuItem::with_id(
                        id,
                        label,
                        item.enabled.unwrap_or(true),
                        item.checked.unwrap_or(false),
                        parse_accel(item),
                    );
                    container.append(&mi).map_err(|e| e.to_string())?;
                    register(
                        id_value.to_string(),
                        RegEntry {
                            item: RegItem::Check(mi),
                            parent: container.clone(),
                            position,
                        },
                    );
                }
            },
            _ => {
                if let Some(role) = &item.role {
                    if let Some(pi) = role_to_predefined(role) {
                        container.append(&pi).map_err(|e| e.to_string())?;
                        if let Some(id) = item.id.as_deref() {
                            register(
                                id.to_string(),
                                RegEntry {
                                    item: RegItem::Predefined(pi),
                                    parent: container.clone(),
                                    position,
                                },
                            );
                        }
                        continue;
                    }
                }
                if let Some(label) = &item.label {
                    let id_value = item.id.as_deref().unwrap_or(label);
                    let id = muda::MenuId::new(id_value);
                    let mi = muda::MenuItem::with_id(
                        id,
                        label,
                        item.enabled.unwrap_or(true),
                        parse_accel(item),
                    );
                    container.append(&mi).map_err(|e| e.to_string())?;
                    register(
                        id_value.to_string(),
                        RegEntry {
                            item: RegItem::MenuItem(mi),
                            parent: container.clone(),
                            position,
                        },
                    );
                }
            },
        }
    }
    Ok(())
}

fn role_to_predefined(role: &str) -> Option<muda::PredefinedMenuItem> {
    use muda::PredefinedMenuItem as P;
    Some(match role {
        "separator" => P::separator(),
        "quit" => P::quit(None),
        "copy" => P::copy(None),
        "cut" => P::cut(None),
        "paste" => P::paste(None),
        "undo" => P::undo(None),
        "redo" => P::redo(None),
        "selectAll" => P::select_all(None),
        "minimize" => P::minimize(None),
        "close" => P::close_window(None),
        "about" => P::about(None::<&str>, None),
        _ => return None,
    })
}

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

    let position: Option<muda::dpi::Position> = match (
        args.get("x").and_then(Value::as_f64),
        args.get("y").and_then(Value::as_f64),
    ) {
        (Some(x), Some(y)) => Some(muda::dpi::LogicalPosition::new(x, y).into()),
        _ => None,
    };

    show_on_window(window, &menu, position)
}

#[cfg(target_os = "windows")]
#[allow(clippy::unnecessary_wraps)]
fn show_on_window(
    window: &tao::window::Window,
    menu: &muda::Menu,
    position: Option<muda::dpi::Position>,
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
#[allow(clippy::unnecessary_wraps)]
fn show_on_window(
    window: &tao::window::Window,
    menu: &muda::Menu,
    position: Option<muda::dpi::Position>,
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
#[allow(clippy::unnecessary_wraps)]
fn show_on_window(
    window: &tao::window::Window,
    menu: &muda::Menu,
    position: Option<muda::dpi::Position>,
) -> Result<(), String> {
    use gtk::glib::object::Cast as _;
    use tao::platform::unix::WindowExtUnix;
    // muda wants `&gtk::Window`; tao hands us `&gtk::ApplicationWindow`,
    // a subclass — upcast_ref is zero-cost.
    let gtk_win: &gtk::Window = window.gtk_window().upcast_ref();
    menu.show_context_menu_for_gtk_window(gtk_win, position);
    Ok(())
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
        _ => Err(format!("menu.{method}: unknown method")),
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
    // We accept both `partial` (preferred — matches the TS API) and a flat
    // shape for backward compat with future raw call sites.
    let partial = args.get("partial").unwrap_or(args);

    if let Some(label) = partial.get("label").and_then(Value::as_str) {
        let label = label.to_string();
        with_bar_entry(id, |entry| {
            bar_set_label(&entry.item, &label);
            Ok(())
        })?;
    }
    if let Some(enabled) = partial.get("enabled").and_then(Value::as_bool) {
        with_bar_entry(id, |entry| {
            bar_set_enabled(&entry.item, enabled);
            Ok(())
        })?;
    }
    if let Some(checked) = partial.get("checked").and_then(Value::as_bool) {
        with_bar_entry(id, |entry| match &entry.item {
            RegItem::Check(c) => {
                c.set_checked(checked);
                Ok(())
            },
            _ => Err(format!(
                "menu.update: item '{id}' is not a checkbox/radio (cannot set 'checked')"
            )),
        })?;
    }
    if let Some(visible) = partial.get("visible").and_then(Value::as_bool) {
        set_visible(id, visible)?;
    }
    Ok(())
}

fn set_visible(id: &str, visible: bool) -> Result<(), String> {
    with_bar_entry(id, |entry| {
        if entry.visible == visible {
            return Ok(());
        }
        if visible {
            // Re-insertion at the original slot, clamped against the parent's
            // current length — siblings hidden in the meantime would otherwise
            // push the index past the end.
            let target = entry.position.min(entry.parent.len());
            entry
                .parent
                .insert(entry.item.as_is_menu_item(), target)
                .map_err(|e| format!("menu.setItemVisible: {e}"))?;
        } else {
            entry
                .parent
                .remove(entry.item.as_is_menu_item())
                .map_err(|e| format!("menu.setItemVisible: {e}"))?;
        }
        entry.visible = visible;
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
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

    /// `build_bar` minus the OS-specific `init_for_hwnd` / `init_for_nsapp`
    /// step — exercises the full builder + registry path without needing
    /// an event loop, so the tests run on every platform under `cargo test`.
    fn build_bar_for_test(items: &[MenuItemDef]) -> muda::Menu {
        build_bar(items).expect("build_bar")
    }

    #[test]
    fn build_bar_registers_top_level_and_children() {
        let items = vec![
            MenuItemDef {
                kind: Some("submenu".to_string()),
                id: Some("file".to_string()),
                label: Some("File".to_string()),
                items: Some(vec![item("file.save", "Save"), item("file.open", "Open")]),
                ..Default::default()
            },
            MenuItemDef {
                kind: Some("submenu".to_string()),
                id: Some("edit".to_string()),
                label: Some("Edit".to_string()),
                items: Some(vec![checkbox("edit.wrap", "Word Wrap", false)]),
                ..Default::default()
            },
        ];
        let _menu = build_bar_for_test(&items);
        BAR_REGISTRY.with(|r| {
            let map = r.borrow();
            assert!(map.contains_key("file"), "top-level submenu registered");
            assert!(map.contains_key("file.save"), "child registered");
            assert!(map.contains_key("file.open"));
            assert!(map.contains_key("edit"));
            assert!(map.contains_key("edit.wrap"));
        });
    }

    #[test]
    fn dispatch_update_label_changes_text() {
        let _menu = build_bar_for_test(&[MenuItemDef {
            kind: Some("submenu".to_string()),
            id: Some("file".to_string()),
            label: Some("File".to_string()),
            items: Some(vec![item("file.save", "Save")]),
            ..Default::default()
        }]);
        dispatch(
            "update",
            &json!({ "id": "file.save", "partial": { "label": "Save As..." } }),
        )
        .expect("update");
        BAR_REGISTRY.with(|r| {
            let map = r.borrow();
            let entry = map.get("file.save").expect("entry");
            if let RegItem::MenuItem(mi) = &entry.item {
                // Windows prepends `&` accelerator hints; assert via `contains`.
                assert!(mi.text().contains("Save As..."), "got: {}", mi.text());
            } else {
                panic!("expected MenuItem");
            }
        });
    }

    #[test]
    fn dispatch_update_unknown_id_errors_clearly() {
        let _menu = build_bar_for_test(&[MenuItemDef {
            kind: Some("submenu".to_string()),
            id: Some("file".to_string()),
            label: Some("File".to_string()),
            items: Some(vec![item("only", "Only")]),
            ..Default::default()
        }]);
        let err = dispatch(
            "update",
            &json!({ "id": "ghost", "partial": { "label": "x" } }),
        )
        .unwrap_err();
        assert!(err.contains("'ghost' not found"), "got: {err}");
    }

    #[test]
    fn dispatch_set_item_checked_rejects_non_checkbox() {
        let _menu = build_bar_for_test(&[MenuItemDef {
            kind: Some("submenu".to_string()),
            id: Some("file".to_string()),
            label: Some("File".to_string()),
            items: Some(vec![item("plain", "Plain"), checkbox("box", "Box", false)]),
            ..Default::default()
        }]);
        // Plain item rejects setItemChecked.
        let err = dispatch(
            "setItemChecked",
            &json!({ "id": "plain", "checked": true }),
        )
        .unwrap_err();
        assert!(err.contains("not a checkbox"), "got: {err}");
        // Checkbox accepts it.
        dispatch("setItemChecked", &json!({ "id": "box", "checked": true })).unwrap();
        BAR_REGISTRY.with(|r| {
            let map = r.borrow();
            if let RegItem::Check(c) = &map.get("box").unwrap().item {
                assert!(c.is_checked());
            }
        });
    }

    #[test]
    fn dispatch_set_item_enabled_flips_state() {
        let _menu = build_bar_for_test(&[MenuItemDef {
            kind: Some("submenu".to_string()),
            id: Some("file".to_string()),
            label: Some("File".to_string()),
            items: Some(vec![item("toggle", "Toggle")]),
            ..Default::default()
        }]);
        BAR_REGISTRY.with(|r| {
            let map = r.borrow();
            if let RegItem::MenuItem(mi) = &map.get("toggle").unwrap().item {
                assert!(mi.is_enabled(), "default enabled");
            }
        });
        dispatch(
            "setItemEnabled",
            &json!({ "id": "toggle", "enabled": false }),
        )
        .unwrap();
        BAR_REGISTRY.with(|r| {
            let map = r.borrow();
            if let RegItem::MenuItem(mi) = &map.get("toggle").unwrap().item {
                assert!(!mi.is_enabled(), "disabled after dispatch");
            }
        });
    }

    #[test]
    fn dispatch_set_item_visible_removes_and_reinserts() {
        let _menu = build_bar_for_test(&[MenuItemDef {
            kind: Some("submenu".to_string()),
            id: Some("file".to_string()),
            label: Some("File".to_string()),
            items: Some(vec![item("a", "A"), item("b", "B"), item("c", "C")]),
            ..Default::default()
        }]);
        // Hide the middle item.
        dispatch(
            "setItemVisible",
            &json!({ "id": "b", "visible": false }),
        )
        .unwrap();
        BAR_REGISTRY.with(|r| {
            let map = r.borrow();
            assert!(!map.get("b").unwrap().visible);
        });
        // Show it back — registry is consistent again.
        dispatch("setItemVisible", &json!({ "id": "b", "visible": true })).unwrap();
        BAR_REGISTRY.with(|r| {
            let map = r.borrow();
            assert!(map.get("b").unwrap().visible);
        });
    }

    #[test]
    fn dispatch_unknown_method_errors() {
        let _menu = build_bar_for_test(&[MenuItemDef {
            kind: Some("submenu".to_string()),
            id: Some("file".to_string()),
            label: Some("File".to_string()),
            items: Some(vec![item("save", "Save")]),
            ..Default::default()
        }]);
        let err = dispatch("nope", &json!({ "id": "save" })).unwrap_err();
        assert!(err.contains("unknown method"), "got: {err}");
    }

    #[test]
    fn dispatch_update_applies_multiple_fields() {
        let _menu = build_bar_for_test(&[MenuItemDef {
            kind: Some("submenu".to_string()),
            id: Some("view".to_string()),
            label: Some("View".to_string()),
            items: Some(vec![checkbox("view.sidebar", "Sidebar", false)]),
            ..Default::default()
        }]);
        dispatch(
            "update",
            &json!({
                "id": "view.sidebar",
                "partial": { "label": "Show Sidebar", "enabled": false, "checked": true }
            }),
        )
        .unwrap();
        BAR_REGISTRY.with(|r| {
            let map = r.borrow();
            let entry = map.get("view.sidebar").unwrap();
            if let RegItem::Check(c) = &entry.item {
                assert!(c.text().contains("Show Sidebar"));
                assert!(!c.is_enabled());
                assert!(c.is_checked());
            }
        });
    }
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
                crate::tynd_log!("menu.init_for_hwnd: {e}");
            }
        }
    }
    #[cfg(target_os = "macos")]
    {
        let _ = window;
        menu.init_for_nsapp();
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        let _ = (menu, window);
    }
}
