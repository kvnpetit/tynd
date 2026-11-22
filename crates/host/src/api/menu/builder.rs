//! Menu item construction shared by menu bars, tray menus, and context menus.

use std::str::FromStr as _;

use muda::accelerator::Accelerator;

use tynd_protocol::runtime::{CheckboxDef, ItemDef, MenuItemDef, MenuRole, RadioDef, SubmenuDef};
/// Muda handles the tray + menu-bar mutation APIs need to look up by id.
#[derive(Clone)]
pub(crate) enum RegItem {
    MenuItem(muda::MenuItem),
    Submenu(muda::Submenu),
    Check(muda::CheckMenuItem),
    Predefined(muda::PredefinedMenuItem),
}

/// Parent container — the build target for `fill_with_registry` and the
/// surface `setItemVisible` calls into. Clone is a refcount bump (Rc-backed).
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

    pub(crate) fn same_as(&self, other: &Self) -> bool {
        match (self, other) {
            (RegContainer::Root(left), RegContainer::Root(right)) => left.id() == right.id(),
            (RegContainer::Sub(left), RegContainer::Sub(right)) => left.id() == right.id(),
            _ => false,
        }
    }
}

/// Per-item record. `position` is the build-time index, used as the
/// re-insert offset for visibility toggles.
pub(crate) struct RegEntry {
    pub(crate) item: RegItem,
    pub(crate) parent: RegContainer,
    pub(crate) position: usize,
}

fn parse_accel_str(s: Option<&str>) -> Option<Accelerator> {
    let s = s?;
    match Accelerator::from_str(s) {
        Ok(a) => Some(a),
        Err(e) => {
            crate::tynd_warn!("menu: bad accelerator {s:?}: {e}");
            None
        },
    }
}

/// Recursively fill a menu container using a caller-provided append closure.
#[cfg(feature = "api-menu")]
pub(crate) fn fill(
    append: &dyn Fn(&dyn muda::IsMenuItem) -> muda::Result<()>,
    items: &[MenuItemDef],
) -> Result<(), String> {
    for item in items {
        match item {
            MenuItemDef::Separator => {
                append(&muda::PredefinedMenuItem::separator()).map_err(|e| e.to_string())?;
            },
            MenuItemDef::Submenu(s) => {
                let label = s.label.as_deref().unwrap_or("");
                let enabled = s.enabled.unwrap_or(true);
                let sub = muda::Submenu::new(label, enabled);
                fill(&|i: &dyn muda::IsMenuItem| sub.append(i), &s.items)?;
                append(&sub).map_err(|e| e.to_string())?;
            },
            MenuItemDef::Checkbox(c) => {
                if let Some(label) = &c.label {
                    let id = muda::MenuId::new(c.id.as_deref().unwrap_or(label));
                    let mi = muda::CheckMenuItem::with_id(
                        id,
                        label,
                        c.enabled.unwrap_or(true),
                        c.checked.unwrap_or(false),
                        parse_accel_str(c.accelerator.as_deref()),
                    );
                    append(&mi).map_err(|e| e.to_string())?;
                }
            },
            MenuItemDef::Radio(r) => {
                // muda has no native radio group — approximate with CheckMenuItem.
                // Apps enforce single-selection by listening for clicks and
                // calling `menu.setChecked(id, bool)` to uncheck siblings.
                if let Some(label) = &r.label {
                    let id = muda::MenuId::new(r.id.as_deref().unwrap_or(label));
                    let mi = muda::CheckMenuItem::with_id(
                        id,
                        label,
                        r.enabled.unwrap_or(true),
                        r.checked.unwrap_or(false),
                        parse_accel_str(r.accelerator.as_deref()),
                    );
                    append(&mi).map_err(|e| e.to_string())?;
                }
            },
            MenuItemDef::Item(i) => {
                if let Some(role) = i.role {
                    if let Some(pi) = role_to_predefined(role) {
                        append(&pi).map_err(|e| e.to_string())?;
                        continue;
                    }
                }
                if let Some(label) = &i.label {
                    let id = muda::MenuId::new(i.id.as_deref().unwrap_or(label));
                    let mi = muda::MenuItem::with_id(
                        id,
                        label,
                        i.enabled.unwrap_or(true),
                        parse_accel_str(i.accelerator.as_deref()),
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
    for item in items {
        let position = container.len();
        append_registered_item(container, item, position, register)?;
    }
    Ok(())
}

fn append_registered_item<F>(
    container: &RegContainer,
    item: &MenuItemDef,
    position: usize,
    register: &mut F,
) -> Result<(), String>
where
    F: FnMut(String, RegEntry),
{
    match item {
        MenuItemDef::Separator => {
            let separator = muda::PredefinedMenuItem::separator();
            container
                .append(&separator)
                .map_err(|error| error.to_string())
        },
        MenuItemDef::Submenu(submenu) => {
            append_registered_submenu(container, submenu, position, register)
        },
        MenuItemDef::Checkbox(CheckboxDef {
            id,
            label,
            enabled,
            checked,
            accelerator,
        })
        | MenuItemDef::Radio(RadioDef {
            id,
            label,
            enabled,
            checked,
            accelerator,
        }) => {
            let Some(label) = label else {
                return Ok(());
            };
            let id_value = id.as_deref().unwrap_or(label);
            let item = muda::CheckMenuItem::with_id(
                muda::MenuId::new(id_value),
                label,
                enabled.unwrap_or(true),
                checked.unwrap_or(false),
                parse_accel_str(accelerator.as_deref()),
            );
            container.append(&item).map_err(|error| error.to_string())?;
            register(
                id_value.to_string(),
                RegEntry {
                    item: RegItem::Check(item),
                    parent: container.clone(),
                    position,
                },
            );
            Ok(())
        },
        MenuItemDef::Item(item) => {
            append_registered_regular_item(container, item, position, register)
        },
    }
}

fn append_registered_submenu<F>(
    container: &RegContainer,
    submenu: &SubmenuDef,
    position: usize,
    register: &mut F,
) -> Result<(), String>
where
    F: FnMut(String, RegEntry),
{
    let label = submenu.label.as_deref().unwrap_or("");
    let enabled = submenu.enabled.unwrap_or(true);
    let id_value = submenu.id.as_deref().unwrap_or(label);
    let item = muda::Submenu::with_id(muda::MenuId::new(id_value), label, enabled);
    fill_with_registry(&RegContainer::Sub(item.clone()), &submenu.items, register)?;
    container.append(&item).map_err(|error| error.to_string())?;
    if let Some(id) = submenu.id.as_deref() {
        register(
            id.to_string(),
            RegEntry {
                item: RegItem::Submenu(item),
                parent: container.clone(),
                position,
            },
        );
    }
    Ok(())
}

fn append_registered_regular_item<F>(
    container: &RegContainer,
    item: &ItemDef,
    position: usize,
    register: &mut F,
) -> Result<(), String>
where
    F: FnMut(String, RegEntry),
{
    if let Some(role) = item.role {
        if let Some(predefined) = role_to_predefined(role) {
            container
                .append(&predefined)
                .map_err(|error| error.to_string())?;
            if let Some(id) = item.id.as_deref() {
                register(
                    id.to_string(),
                    RegEntry {
                        item: RegItem::Predefined(predefined),
                        parent: container.clone(),
                        position,
                    },
                );
            }
            return Ok(());
        }
    }

    let Some(label) = &item.label else {
        return Ok(());
    };
    let id_value = item.id.as_deref().unwrap_or(label);
    let menu_item = muda::MenuItem::with_id(
        muda::MenuId::new(id_value),
        label,
        item.enabled.unwrap_or(true),
        parse_accel_str(item.accelerator.as_deref()),
    );
    container
        .append(&menu_item)
        .map_err(|error| error.to_string())?;
    register(
        id_value.to_string(),
        RegEntry {
            item: RegItem::MenuItem(menu_item),
            parent: container.clone(),
            position,
        },
    );
    Ok(())
}
fn role_to_predefined(role: MenuRole) -> Option<muda::PredefinedMenuItem> {
    use muda::PredefinedMenuItem as P;
    Some(match role {
        MenuRole::Separator => P::separator(),
        MenuRole::Quit => P::quit(None),
        MenuRole::Copy => P::copy(None),
        MenuRole::Cut => P::cut(None),
        MenuRole::Paste => P::paste(None),
        MenuRole::Undo => P::undo(None),
        MenuRole::Redo => P::redo(None),
        MenuRole::SelectAll => P::select_all(None),
        MenuRole::Minimize => P::minimize(None),
        MenuRole::Close => P::close_window(None),
        MenuRole::About => P::about(None::<&str>, None),
    })
}
