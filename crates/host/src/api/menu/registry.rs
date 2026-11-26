//! Id registry of one native menu: the muda root plus an `id -> handle` map,
//! so runtime mutations (`setItemEnabled`, `append`, `remove`, …) reach the
//! native items without a rebuild and radio groups stay exclusive. Shared by
//! the application menu bar, tray menus, per-window menu bars and the Dock
//! menu.
// `api-tray` without `api-menu` compiles this for tray menus only.
#![cfg_attr(not(feature = "api-menu"), allow(dead_code))]

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::{Rc, Weak},
};

use serde_json::Value;

use tynd_protocol::runtime::{MenuIcon, MenuItemDef};

use super::builder::{
    fill_with_registry, insert_with_registry, parse_accelerator, resolve_icon, ItemIcon,
    RegContainer, RegEntry, RegItem,
};

/// Which API a registry serves: the error prefix, the name of its item
/// removal method and whether top-level entries must be submenus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Scope {
    name: &'static str,
    remove_method: &'static str,
    submenus_only: bool,
}

impl Scope {
    /// Menu bars (`menu.*`, `win.setMenu`): top-level entries are submenus.
    pub(crate) const BAR: Self = Self {
        name: "menu",
        remove_method: "remove",
        submenus_only: true,
    };
    /// The Dock menu: a flat item list.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub(crate) const DOCK: Self = Self {
        name: "menu",
        remove_method: "remove",
        submenus_only: false,
    };
    /// Tray menus (`tray.*`; `tray.remove` removes the icon itself).
    #[cfg_attr(not(feature = "api-tray"), allow(dead_code))]
    pub(crate) const TRAY: Self = Self {
        name: "tray",
        remove_method: "removeItem",
        submenus_only: false,
    };
}

/// Per-id metadata. `position` is the build-time index inside the parent, the
/// re-insert offset when a hidden item is shown again. `checked` is the last
/// known state of a checkbox / radio: muda flips it before reporting a click,
/// which tells which menu a click on a shared id came from.
struct Entry {
    item: RegItem,
    parent: RegContainer,
    position: usize,
    visible: bool,
    radio: bool,
    accelerator: Option<String>,
    checked: Cell<bool>,
}

impl From<RegEntry> for Entry {
    fn from(entry: RegEntry) -> Self {
        let checked = match &entry.item {
            RegItem::Check(item) => item.is_checked(),
            _ => false,
        };
        Self {
            item: entry.item,
            parent: entry.parent,
            position: entry.position,
            visible: true,
            radio: entry.radio,
            accelerator: entry.accelerator,
            checked: Cell::new(checked),
        }
    }
}

pub(crate) struct MenuRegistry {
    scope: Scope,
    root: muda::Menu,
    entries: HashMap<String, Entry>,
}

impl MenuRegistry {
    /// Build a fresh menu. A bad item (icon, accelerator…) fails the whole
    /// build, so a caller replacing a live menu keeps the old one on error.
    pub(crate) fn build(scope: Scope, items: &[MenuItemDef]) -> Result<Self, String> {
        Self::fill(scope, muda::Menu::new(), items)
    }

    /// Fill `root` (empty) with `items` and register them.
    pub(crate) fn fill(
        scope: Scope,
        root: muda::Menu,
        items: &[MenuItemDef],
    ) -> Result<Self, String> {
        let mut entries = HashMap::new();
        fill_with_registry(
            &RegContainer::Root(root.clone()),
            items,
            &mut |id, entry| {
                entries.insert(id, Entry::from(entry));
            },
        )?;
        Ok(Self {
            scope,
            root,
            entries,
        })
    }

    pub(crate) fn root(&self) -> &muda::Menu {
        &self.root
    }

    #[cfg(all(test, not(target_os = "macos")))]
    pub(crate) fn contains(&self, id: &str) -> bool {
        self.entries.contains_key(id)
    }

    /// Checked state of checkbox / radio `id`.
    #[cfg(all(test, not(target_os = "macos")))]
    pub(crate) fn is_checked(&self, id: &str) -> Option<bool> {
        match &self.entries.get(id)?.item {
            RegItem::Check(item) => Some(item.is_checked()),
            _ => None,
        }
    }

    /// Simulate muda's click on check item `id`: flip it, then report it.
    #[cfg(all(test, not(target_os = "macos")))]
    pub(crate) fn click(&self, id: &str) -> bool {
        if let Some(RegItem::Check(item)) = self.entries.get(id).map(|entry| &entry.item) {
            item.set_checked(!item.is_checked());
        }
        self.on_activated(id)
    }

    #[cfg(all(test, not(target_os = "macos")))]
    pub(crate) fn is_submenu(&self, id: &str) -> bool {
        matches!(
            self.entries.get(id).map(|entry| &entry.item),
            Some(RegItem::Submenu(_))
        )
    }

    #[cfg(all(test, not(target_os = "macos")))]
    pub(crate) fn is_icon_item(&self, id: &str) -> bool {
        matches!(
            self.entries.get(id).map(|entry| &entry.item),
            Some(RegItem::Icon(_))
        )
    }

    #[cfg(all(test, not(target_os = "macos")))]
    pub(crate) fn accelerator(&self, id: &str) -> Option<&str> {
        self.entries.get(id)?.accelerator.as_deref()
    }

    /// Item mutation `method`, or `None` when `method` is not an item method.
    pub(crate) fn dispatch(&mut self, method: &str, args: &Value) -> Option<Result<(), String>> {
        let name = self.scope.name;
        Some(match method {
            "update" => self
                .require_id(args, "update")
                .and_then(|id| self.update(&id, args)),
            "setItemEnabled" => self.require_id(args, "setItemEnabled").and_then(|id| {
                let enabled = self.require_bool(args, "setItemEnabled", "enabled")?;
                self.with_entry(&id, |entry| {
                    set_enabled(&entry.item, enabled);
                    Ok(())
                })
            }),
            "setItemLabel" => self.require_id(args, "setItemLabel").and_then(|id| {
                let label = args
                    .get("label")
                    .and_then(Value::as_str)
                    .ok_or_else(|| format!("{name}.setItemLabel: missing 'label'"))?;
                self.with_entry(&id, |entry| {
                    set_label(&entry.item, label);
                    Ok(())
                })
            }),
            "setItemChecked" => self.require_id(args, "setItemChecked").and_then(|id| {
                let checked = self.require_bool(args, "setItemChecked", "checked")?;
                self.set_checked("setItemChecked", &id, checked)
            }),
            "setItemVisible" => self.require_id(args, "setItemVisible").and_then(|id| {
                let visible = self.require_bool(args, "setItemVisible", "visible")?;
                self.set_visible(&id, visible)
            }),
            "setItemAccelerator" => self.require_id(args, "setItemAccelerator").and_then(|id| {
                let accelerator = match args.get("accelerator") {
                    None | Some(Value::Null) => None,
                    Some(Value::String(text)) => Some(text.clone()),
                    Some(_) => {
                        return Err(format!(
                            "{name}.setItemAccelerator: 'accelerator' must be a string or null"
                        ))
                    },
                };
                self.set_accelerator(&id, accelerator)
            }),
            "setItemIcon" => self.require_id(args, "setItemIcon").and_then(|id| {
                let icon = optional_field::<MenuIcon>(name, args, "setItemIcon", "icon")?;
                let native = optional_field::<String>(name, args, "setItemIcon", "nativeIcon")?;
                let icon = resolve_icon(icon.as_ref(), native.as_deref())
                    .map_err(|error| format!("{name}.setItemIcon: {error}"))?;
                self.set_icon(&id, icon)
            }),
            "append" => self.append(args),
            method if method == self.scope.remove_method => self
                .require_id(args, method)
                .and_then(|id| self.remove(method, &id)),
            _ => return None,
        })
    }

    /// Click hook: muda toggles a check item on click, so a clicked radio is
    /// forced back to checked and the rest of its group is cleared. Returns
    /// whether the click came from this menu: `id` is registered here and,
    /// for a checkbox / radio, its state flipped (another menu may hold an
    /// item with the same id).
    pub(crate) fn on_activated(&self, id: &str) -> bool {
        let Some(entry) = self.entries.get(id) else {
            return false;
        };
        let RegItem::Check(item) = &entry.item else {
            return true;
        };
        let checked = item.is_checked();
        if checked == entry.checked.get() {
            return false;
        }
        if entry.radio {
            item.set_checked(true);
            entry.checked.set(true);
            self.uncheck_radio_siblings(id, entry);
        } else {
            entry.checked.set(checked);
        }
        true
    }

    fn not_found(&self, id: &str) -> String {
        format!("{}: item id '{id}' not found", self.scope.name)
    }

    fn require_id(&self, args: &Value, op: &str) -> Result<String, String> {
        args.get("id")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| format!("{}.{op}: missing 'id'", self.scope.name))
    }

    fn require_bool(&self, args: &Value, op: &str, field: &str) -> Result<bool, String> {
        args.get(field)
            .and_then(Value::as_bool)
            .ok_or_else(|| format!("{}.{op}: missing '{field}'", self.scope.name))
    }

    fn with_entry<T>(
        &mut self,
        id: &str,
        f: impl FnOnce(&mut Entry) -> Result<T, String>,
    ) -> Result<T, String> {
        let missing = self.not_found(id);
        let entry = self.entries.get_mut(id).ok_or(missing)?;
        f(entry)
    }

    /// `update(id, { label?, enabled?, checked?, visible? })` in one call.
    fn update(&mut self, id: &str, args: &Value) -> Result<(), String> {
        let name = self.scope.name;
        let partial = args
            .get("partial")
            .filter(|partial| partial.is_object())
            .ok_or_else(|| format!("{name}.update: missing 'partial' object"))?;
        let checked = partial.get("checked").and_then(Value::as_bool);
        if checked.is_some() {
            self.with_entry(id, |entry| match entry.item {
                RegItem::Check(_) => Ok(()),
                _ => Err(format!(
                    "{name}.update: item '{id}' is not a checkbox/radio (cannot set 'checked')"
                )),
            })?;
        }
        // Visibility is the only fallible native mutation: apply it first so
        // an error cannot leave a half-updated item.
        if let Some(visible) = partial.get("visible").and_then(Value::as_bool) {
            self.set_visible(id, visible)?;
        }
        let label = partial.get("label").and_then(Value::as_str);
        let enabled = partial.get("enabled").and_then(Value::as_bool);
        self.with_entry(id, |entry| {
            if let Some(label) = label {
                set_label(&entry.item, label);
            }
            if let Some(enabled) = enabled {
                set_enabled(&entry.item, enabled);
            }
            Ok(())
        })?;
        if let Some(checked) = checked {
            self.set_checked("update", id, checked)?;
        }
        Ok(())
    }

    fn set_checked(&self, op: &str, id: &str, checked: bool) -> Result<(), String> {
        let entry = self.entries.get(id).ok_or_else(|| self.not_found(id))?;
        let RegItem::Check(item) = &entry.item else {
            return Err(format!(
                "{}.{op}: item '{id}' is not a checkbox/radio",
                self.scope.name
            ));
        };
        item.set_checked(checked);
        entry.checked.set(checked);
        if checked && entry.radio {
            self.uncheck_radio_siblings(id, entry);
        }
        Ok(())
    }

    /// Uncheck the other radios of `id`'s group: the run of adjacent radio
    /// items around it in the same parent (separators or other items end it).
    fn uncheck_radio_siblings(&self, id: &str, entry: &Entry) {
        let siblings = entry.parent.child_ids();
        let Some(index) = siblings.iter().position(|sibling| sibling.0 == id) else {
            return;
        };
        let radio_at = |position: usize| {
            self.entries
                .get(&siblings[position].0)
                .filter(|sibling| sibling.radio && sibling.parent.same_as(&entry.parent))
        };
        let mut start = index;
        while start > 0 && radio_at(start - 1).is_some() {
            start -= 1;
        }
        let mut end = index;
        while end + 1 < siblings.len() && radio_at(end + 1).is_some() {
            end += 1;
        }
        for position in (start..=end).filter(|&position| position != index) {
            if let Some(sibling) = radio_at(position) {
                if let RegItem::Check(item) = &sibling.item {
                    item.set_checked(false);
                    sibling.checked.set(false);
                }
            }
        }
    }

    fn set_visible(&mut self, id: &str, visible: bool) -> Result<(), String> {
        let name = self.scope.name;
        let (position, parent, already_visible) = self
            .entries
            .get(id)
            .map(|entry| (entry.position, entry.parent.clone(), entry.visible))
            .ok_or_else(|| self.not_found(id))?;
        if already_visible == visible {
            return Ok(());
        }
        let hidden_before = self
            .entries
            .values()
            .filter(|entry| {
                !entry.visible && entry.position < position && entry.parent.same_as(&parent)
            })
            .count();
        self.with_entry(id, |entry| {
            if visible {
                let target = position.saturating_sub(hidden_before).min(parent.len());
                parent.insert(entry.item.as_is_menu_item(), target)
            } else {
                parent.remove(entry.item.as_is_menu_item())
            }
            .map_err(|error| format!("{name}.setItemVisible: {error}"))?;
            entry.visible = visible;
            Ok(())
        })
    }

    fn set_accelerator(&mut self, id: &str, accelerator: Option<String>) -> Result<(), String> {
        let name = self.scope.name;
        let parsed = accelerator
            .as_deref()
            .map(parse_accelerator)
            .transpose()
            .map_err(|error| format!("{name}.setItemAccelerator: {error}"))?;
        self.with_entry(id, |entry| {
            match &entry.item {
                RegItem::MenuItem(item) => item.set_accelerator(parsed),
                RegItem::Icon(item) => item.set_accelerator(parsed),
                RegItem::Check(item) => item.set_accelerator(parsed),
                RegItem::Submenu(_) | RegItem::Predefined(_) => {
                    return Err(format!(
                        "{name}.setItemAccelerator: item '{id}' is a submenu or role item and has no accelerator"
                    ))
                },
            }
            .map_err(|error| format!("{name}.setItemAccelerator: {error}"))?;
            entry.accelerator = accelerator;
            Ok(())
        })
    }

    fn set_icon(&mut self, id: &str, icon: ItemIcon) -> Result<(), String> {
        let name = self.scope.name;
        self.with_entry(id, |entry| {
            match (&entry.item, icon) {
                (RegItem::Icon(item), ItemIcon::None) => item.set_icon(None),
                (RegItem::Icon(item), ItemIcon::Custom(icon)) => item.set_icon(Some(icon)),
                (RegItem::Icon(item), ItemIcon::Native(icon)) => item.set_native_icon(Some(icon)),
                (RegItem::Submenu(item), ItemIcon::None) => item.set_icon(None),
                (RegItem::Submenu(item), ItemIcon::Custom(icon)) => item.set_icon(Some(icon)),
                (RegItem::Submenu(item), ItemIcon::Native(icon)) => {
                    item.set_native_icon(Some(icon));
                },
                (RegItem::MenuItem(_), ItemIcon::None) => {},
                (RegItem::MenuItem(item), icon) => {
                    // muda keeps icons on a separate item type: swap the plain
                    // item for an icon item with the same id, text and state.
                    let replacement = icon_item_from(item, entry.accelerator.as_deref(), icon);
                    if entry.visible {
                        let index = entry
                            .parent
                            .child_ids()
                            .iter()
                            .position(|child| child == item.id())
                            .unwrap_or_else(|| entry.parent.len());
                        entry
                            .parent
                            .insert(&replacement, index)
                            .and_then(|()| entry.parent.remove(item))
                            .map_err(|error| format!("{name}.setItemIcon: {error}"))?;
                    }
                    entry.item = RegItem::Icon(replacement);
                },
                (RegItem::Check(_) | RegItem::Predefined(_), _) => {
                    return Err(format!(
                        "{name}.setItemIcon: item '{id}' is a checkbox/radio or role item, which cannot show an icon"
                    ))
                },
            }
            Ok(())
        })
    }

    /// `append({ item, parent?, index? })`: add an item (or a whole submenu)
    /// to the menu or to one of its submenus, optionally at `index`.
    fn append(&mut self, args: &Value) -> Result<(), String> {
        let name = self.scope.name;
        let item: MenuItemDef = optional_field(name, args, "append", "item")?
            .ok_or_else(|| format!("{name}.append: missing 'item'"))?;
        let container = match optional_field::<String>(name, args, "append", "parent")? {
            None => {
                if self.scope.submenus_only && !matches!(item, MenuItemDef::Submenu(_)) {
                    return Err(format!(
                        "{name}.append: top-level bar entries must be submenus ({{ type: \"submenu\", label, items }}) — pass `parent` to add an item to a submenu"
                    ));
                }
                RegContainer::Root(self.root.clone())
            },
            Some(parent) => self.with_entry(&parent, |entry| match &entry.item {
                RegItem::Submenu(submenu) => Ok(RegContainer::Sub(submenu.clone())),
                _ => Err(format!("{name}.append: parent '{parent}' is not a submenu")),
            })?,
        };
        let index = match args.get("index") {
            None | Some(Value::Null) => None,
            Some(value) => {
                let index = value
                    .as_u64()
                    .and_then(|index| usize::try_from(index).ok())
                    .ok_or_else(|| {
                        format!("{name}.append: 'index' must be a non-negative integer")
                    })?;
                let len = container.len();
                if index > len {
                    return Err(format!(
                        "{name}.append: 'index' {index} is out of range (the parent has {len} items)"
                    ));
                }
                Some(index)
            },
        };
        let mut ids = Vec::new();
        explicit_ids(&item, &mut ids);
        if let Some(id) = ids.iter().find(|id| self.entries.contains_key(**id)) {
            return Err(format!("{name}.append: item id '{id}' already exists"));
        }
        let mut added = Vec::new();
        insert_with_registry(&container, &item, index, &mut |id, entry| {
            added.push((id, Entry::from(entry)));
        })
        .map_err(|error| format!("{name}.append: {error}"))?;
        self.entries.extend(added);
        Ok(())
    }

    /// Detach an item and forget it (and, for a submenu, every registered
    /// descendant).
    fn remove(&mut self, op: &str, id: &str) -> Result<(), String> {
        let entry = self.entries.get(id).ok_or_else(|| self.not_found(id))?;
        if entry.visible {
            entry
                .parent
                .remove(entry.item.as_is_menu_item())
                .map_err(|error| format!("{}.{op}: {error}", self.scope.name))?;
        }
        let mut pending = vec![id.to_string()];
        while let Some(current) = pending.pop() {
            let Some(removed) = self.entries.remove(&current) else {
                continue;
            };
            if let RegItem::Submenu(submenu) = removed.item {
                let container = RegContainer::Sub(submenu);
                pending.extend(
                    self.entries
                        .iter()
                        .filter(|(_, child)| child.parent.same_as(&container))
                        .map(|(child_id, _)| child_id.clone()),
                );
            }
        }
        Ok(())
    }
}

thread_local! {
    /// Menus without a dedicated owner lookup (per-window bars, the Dock
    /// menu), for click routing; each lives as long as its owner holds it.
    static TRACKED: RefCell<Vec<Weak<MenuRegistry>>> = const { RefCell::new(Vec::new()) };
}

/// Route clicks on `registry`'s items to it while it is alive (main thread).
pub(crate) fn track(registry: &Rc<MenuRegistry>) {
    let _ = TRACKED.try_with(|tracked| {
        if let Ok(mut tracked) = tracked.try_borrow_mut() {
            tracked.retain(|weak| weak.strong_count() > 0);
            tracked.push(Rc::downgrade(registry));
        }
    });
}

/// Click hook for the tracked menus: every one the click may have come from
/// handles it. Returns how many did (more than one: an ambiguous plain id).
pub(crate) fn activate_tracked(id: &str) -> usize {
    TRACKED
        .try_with(|tracked| {
            let live: Vec<Rc<MenuRegistry>> = match tracked.try_borrow() {
                Ok(tracked) => tracked.iter().filter_map(Weak::upgrade).collect(),
                Err(_) => return 0,
            };
            live.iter()
                .filter(|registry| registry.on_activated(id))
                .count()
        })
        .unwrap_or(0)
}

pub(crate) fn optional_field<T: serde::de::DeserializeOwned>(
    scope: &str,
    args: &Value,
    op: &str,
    field: &str,
) -> Result<Option<T>, String> {
    match args.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => serde_json::from_value(value.clone())
            .map(Some)
            .map_err(|error| format!("{scope}.{op}: invalid '{field}' — {error}")),
    }
}

fn explicit_ids<'a>(item: &'a MenuItemDef, out: &mut Vec<&'a str>) {
    if let Some(id) = item.id() {
        out.push(id);
    }
    if let MenuItemDef::Submenu(submenu) = item {
        for child in &submenu.items {
            explicit_ids(child, out);
        }
    }
}

fn set_enabled(item: &RegItem, enabled: bool) {
    match item {
        RegItem::MenuItem(i) => i.set_enabled(enabled),
        RegItem::Submenu(i) => i.set_enabled(enabled),
        RegItem::Check(i) => i.set_enabled(enabled),
        RegItem::Icon(i) => i.set_enabled(enabled),
        // OS-managed roles ("quit", "copy") have no setter: ignored so callers
        // need not type-check before toggling.
        RegItem::Predefined(_) => {},
    }
}

fn set_label(item: &RegItem, label: &str) {
    match item {
        RegItem::MenuItem(i) => i.set_text(label),
        RegItem::Submenu(i) => i.set_text(label),
        RegItem::Check(i) => i.set_text(label),
        RegItem::Predefined(i) => i.set_text(label),
        RegItem::Icon(i) => i.set_text(label),
    }
}

fn icon_item_from(
    item: &muda::MenuItem,
    accelerator: Option<&str>,
    icon: ItemIcon,
) -> muda::IconMenuItem {
    let accelerator = accelerator.and_then(|text| parse_accelerator(text).ok());
    let (id, text, enabled) = (item.id().clone(), item.text(), item.is_enabled());
    match icon {
        ItemIcon::Native(icon) => {
            muda::IconMenuItem::with_id_and_native_icon(id, text, enabled, Some(icon), accelerator)
        },
        ItemIcon::Custom(icon) => {
            muda::IconMenuItem::with_id(id, text, enabled, Some(icon), accelerator)
        },
        ItemIcon::None => muda::IconMenuItem::with_id(id, text, enabled, None, accelerator),
    }
}

// AppKit requires the main thread; macOS exercises the menu in smoke-packaged.ts.
#[cfg(all(test, not(target_os = "macos")))]
mod tests {
    use serde_json::json;

    use super::*;

    fn menu(scope: Scope, items: Value) -> MenuRegistry {
        let items: Vec<MenuItemDef> = serde_json::from_value(items).unwrap();
        MenuRegistry::build(scope, &items).unwrap()
    }

    fn sizes() -> Value {
        json!([
            { "type": "radio", "id": "small", "label": "Small", "checked": true },
            { "type": "radio", "id": "large", "label": "Large" },
            { "type": "checkbox", "id": "wrap", "label": "Wrap" }
        ])
    }

    #[test]
    fn a_click_is_handled_by_the_menu_it_came_from() {
        // Two menus share the ids: only the one whose item muda flipped
        // reacts, and its radio group stays exclusive.
        let first = menu(Scope::DOCK, sizes());
        let second = menu(Scope::TRAY, sizes());
        assert!(second.click("large"));
        assert!(!first.on_activated("large"));
        assert_eq!(second.is_checked("large"), Some(true));
        assert_eq!(second.is_checked("small"), Some(false));
        assert_eq!(first.is_checked("small"), Some(true));
        assert_eq!(first.is_checked("large"), Some(false));

        // Clicking the checked radio keeps it checked.
        assert!(second.click("large"));
        assert_eq!(second.is_checked("large"), Some(true));

        assert!(first.click("wrap"));
        assert_eq!(first.is_checked("wrap"), Some(true));
        assert!(!second.on_activated("wrap"));
        assert!(!first.on_activated("unknown"));
    }

    #[test]
    fn tracked_menus_route_clicks_while_alive() {
        let window_bar = Rc::new(menu(
            Scope::BAR,
            json!([{ "type": "submenu", "label": "View", "items": sizes() }]),
        ));
        track(&window_bar);
        if let Some(RegItem::Check(item)) = window_bar.entries.get("large").map(|e| &e.item) {
            item.set_checked(true);
        }
        assert_eq!(activate_tracked("large"), 1);
        assert_eq!(window_bar.is_checked("small"), Some(false));
        drop(window_bar);
        assert_eq!(activate_tracked("large"), 0);
    }

    #[test]
    fn a_plain_id_held_by_several_tracked_menus_is_counted_by_each() {
        let items = json!([{ "type": "submenu", "label": "File", "items": [
            { "id": "shared.quit", "label": "Quit" }
        ] }]);
        let first = Rc::new(menu(Scope::BAR, items.clone()));
        let second = Rc::new(menu(Scope::BAR, items));
        track(&first);
        track(&second);
        assert_eq!(activate_tracked("shared.quit"), 2);
        drop(second);
        assert_eq!(activate_tracked("shared.quit"), 1);
    }

    #[test]
    fn bar_scope_requires_top_level_submenus() {
        let mut bar = menu(
            Scope::BAR,
            json!([{ "type": "submenu", "id": "view", "label": "View", "items": [] }]),
        );
        let error = bar
            .dispatch("append", &json!({ "item": { "id": "x", "label": "X" } }))
            .unwrap()
            .unwrap_err();
        assert!(error.starts_with("menu.append:"), "{error}");
        assert!(bar.is_submenu("view"));
        assert!(bar
            .dispatch("removeItem", &json!({ "id": "view" }))
            .is_none());
        bar.dispatch("remove", &json!({ "id": "view" }))
            .unwrap()
            .unwrap();
        assert!(!bar.contains("view"));
    }
}
