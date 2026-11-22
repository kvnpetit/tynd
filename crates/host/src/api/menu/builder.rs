//! Menu item construction shared by menu bars, tray menus, and context menus.

use std::str::FromStr as _;

use base64::{engine::general_purpose::STANDARD, Engine as _};
use muda::accelerator::Accelerator;

use tynd_protocol::runtime::{
    CheckboxDef, ItemDef, MenuIcon, MenuItemDef, MenuRole, RadioDef, SubmenuDef,
};

/// Largest accepted icon edge, in pixels. Menu icons render at 16–32 px, so
/// anything bigger is a mistake (or an attempt to make the host decode a
/// huge image).
const MAX_ICON_EDGE: u32 = 512;
/// Largest accepted base64 icon payload, in decoded bytes.
const MAX_ICON_BYTES: usize = 1024 * 1024;

/// Muda handles the tray + menu-bar mutation APIs need to look up by id.
#[derive(Clone)]
pub(crate) enum RegItem {
    MenuItem(muda::MenuItem),
    Submenu(muda::Submenu),
    Check(muda::CheckMenuItem),
    Predefined(muda::PredefinedMenuItem),
    Icon(muda::IconMenuItem),
}

impl RegItem {
    pub(crate) fn as_is_menu_item(&self) -> &dyn muda::IsMenuItem {
        match self {
            RegItem::MenuItem(i) => i,
            RegItem::Submenu(i) => i,
            RegItem::Check(i) => i,
            RegItem::Predefined(i) => i,
            RegItem::Icon(i) => i,
        }
    }
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

    /// Ids of the attached children, in display order.
    pub(crate) fn child_ids(&self) -> Vec<muda::MenuId> {
        let items = match self {
            RegContainer::Root(m) => m.items(),
            RegContainer::Sub(s) => s.items(),
        };
        items.iter().map(|item| item.id().clone()).collect()
    }

    pub(crate) fn same_as(&self, other: &Self) -> bool {
        match (self, other) {
            (RegContainer::Root(left), RegContainer::Root(right)) => left.id() == right.id(),
            (RegContainer::Sub(left), RegContainer::Sub(right)) => left.id() == right.id(),
            _ => false,
        }
    }

    /// Append `item`, or insert it at `at` when given.
    fn place(&self, item: &dyn muda::IsMenuItem, at: Option<usize>) -> Result<(), String> {
        match at {
            Some(position) => self.insert(item, position),
            None => self.append(item),
        }
        .map_err(|error| error.to_string())
    }
}

/// Per-item record. `position` is the build-time index, used as the
/// re-insert offset for visibility toggles.
pub(crate) struct RegEntry {
    pub(crate) item: RegItem,
    pub(crate) parent: RegContainer,
    pub(crate) position: usize,
    /// `type: "radio"` item: a click keeps exactly one item of its adjacent
    /// radio run checked.
    pub(crate) radio: bool,
    /// Accelerator the item was built with (muda has no getter), so an item
    /// can be rebuilt with an icon without losing its shortcut.
    pub(crate) accelerator: Option<String>,
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

/// Strict accelerator parse for runtime mutations: a typo is reported to the
/// caller instead of being logged and dropped.
pub(crate) fn parse_accelerator(s: &str) -> Result<Accelerator, String> {
    Accelerator::from_str(s).map_err(|error| {
        format!("invalid accelerator {s:?}: {error} (expected e.g. \"CmdOrCtrl+Shift+S\")")
    })
}

fn check_icon_size(width: u32, height: u32) -> Result<(), String> {
    if width == 0 || height == 0 || width > MAX_ICON_EDGE || height > MAX_ICON_EDGE {
        return Err(format!(
            "menu icon must be between 1x1 and {MAX_ICON_EDGE}x{MAX_ICON_EDGE} pixels, got {width}x{height}"
        ));
    }
    Ok(())
}

fn decode_base64(field: &str, data: &str) -> Result<Vec<u8>, String> {
    // Reject oversized payloads before decoding (base64 inflates by 4/3).
    if data.len() / 4 * 3 > MAX_ICON_BYTES {
        return Err(format!(
            "menu icon '{field}' exceeds {MAX_ICON_BYTES} bytes"
        ));
    }
    STANDARD
        .decode(data)
        .map_err(|error| format!("menu icon '{field}' is not valid base64: {error}"))
}

/// Decode a [`MenuIcon`] into raw RGBA pixels of a validated size. File
/// paths go through the fs allow-list (`platform::icon::load_rgba`).
pub(crate) fn icon_rgba(icon: &MenuIcon) -> Result<(Vec<u8>, u32, u32), String> {
    let (rgba, width, height) = match icon {
        MenuIcon::Path(path) => crate::platform::icon::load_rgba(path)?,
        MenuIcon::Png { png } => {
            let bytes = decode_base64("png", png)?;
            let image = image::load_from_memory_with_format(&bytes, image::ImageFormat::Png)
                .map_err(|error| format!("menu icon 'png' is not a valid PNG: {error}"))?
                .into_rgba8();
            let (width, height) = image.dimensions();
            (image.into_raw(), width, height)
        },
        MenuIcon::Rgba {
            rgba,
            width,
            height,
        } => {
            check_icon_size(*width, *height)?;
            let bytes = decode_base64("rgba", rgba)?;
            let expected = u64::from(*width) * u64::from(*height) * 4;
            if bytes.len() as u64 != expected {
                return Err(format!(
                    "menu icon 'rgba' has {} bytes, expected width * height * 4 = {expected}",
                    bytes.len()
                ));
            }
            (bytes, *width, *height)
        },
    };
    check_icon_size(width, height)?;
    Ok((rgba, width, height))
}

/// Load a [`MenuIcon`] into a muda icon.
pub(crate) fn load_icon(icon: &MenuIcon) -> Result<muda::Icon, String> {
    let (rgba, width, height) = icon_rgba(icon)?;
    muda::Icon::from_rgba(rgba, width, height).map_err(|error| format!("menu icon: {error}"))
}

/// Map a camelCase native icon name (`"add"`, `"folder"`, `"trashFull"`) to
/// [`muda::NativeIcon`]. macOS uses the AppKit image, Windows the shell stock
/// icon and Linux the freedesktop theme icon where an equivalent exists.
pub(crate) fn parse_native_icon(name: &str) -> Result<muda::NativeIcon, String> {
    use muda::NativeIcon as N;
    Ok(match name {
        "add" => N::Add,
        "advanced" => N::Advanced,
        "bluetooth" => N::Bluetooth,
        "bookmarks" => N::Bookmarks,
        "caution" => N::Caution,
        "colorPanel" => N::ColorPanel,
        "columnView" => N::ColumnView,
        "computer" => N::Computer,
        "enterFullScreen" => N::EnterFullScreen,
        "everyone" => N::Everyone,
        "exitFullScreen" => N::ExitFullScreen,
        "flowView" => N::FlowView,
        "folder" => N::Folder,
        "folderBurnable" => N::FolderBurnable,
        "folderSmart" => N::FolderSmart,
        "followLinkFreestanding" => N::FollowLinkFreestanding,
        "fontPanel" => N::FontPanel,
        "goLeft" => N::GoLeft,
        "goRight" => N::GoRight,
        "home" => N::Home,
        "iChatTheater" => N::IChatTheater,
        "iconView" => N::IconView,
        "info" => N::Info,
        "invalidDataFreestanding" => N::InvalidDataFreestanding,
        "leftFacingTriangle" => N::LeftFacingTriangle,
        "listView" => N::ListView,
        "lockLocked" => N::LockLocked,
        "lockUnlocked" => N::LockUnlocked,
        "menuMixedState" => N::MenuMixedState,
        "menuOnState" => N::MenuOnState,
        "mobileMe" => N::MobileMe,
        "multipleDocuments" => N::MultipleDocuments,
        "network" => N::Network,
        "path" => N::Path,
        "preferencesGeneral" => N::PreferencesGeneral,
        "quickLook" => N::QuickLook,
        "refreshFreestanding" => N::RefreshFreestanding,
        "refresh" => N::Refresh,
        "remove" => N::Remove,
        "revealFreestanding" => N::RevealFreestanding,
        "rightFacingTriangle" => N::RightFacingTriangle,
        "share" => N::Share,
        "slideshow" => N::Slideshow,
        "smartBadge" => N::SmartBadge,
        "statusAvailable" => N::StatusAvailable,
        "statusNone" => N::StatusNone,
        "statusPartiallyAvailable" => N::StatusPartiallyAvailable,
        "statusUnavailable" => N::StatusUnavailable,
        "stopProgressFreestanding" => N::StopProgressFreestanding,
        "stopProgress" => N::StopProgress,
        "trashEmpty" => N::TrashEmpty,
        "trashFull" => N::TrashFull,
        "user" => N::User,
        "userAccounts" => N::UserAccounts,
        "userGroup" => N::UserGroup,
        "userGuest" => N::UserGuest,
        _ => {
            return Err(format!(
                "unknown native icon {name:?} (expected a camelCase name such as \"add\", \"folder\" or \"trashFull\")"
            ))
        },
    })
}

/// Resolved icon of an item or submenu.
pub(crate) enum ItemIcon {
    None,
    Custom(muda::Icon),
    Native(muda::NativeIcon),
}

pub(crate) fn resolve_icon(
    icon: Option<&MenuIcon>,
    native_icon: Option<&str>,
) -> Result<ItemIcon, String> {
    match (icon, native_icon) {
        (Some(_), Some(_)) => Err("menu: set either 'icon' or 'nativeIcon', not both".into()),
        (Some(icon), None) => Ok(ItemIcon::Custom(load_icon(icon)?)),
        (None, Some(name)) => Ok(ItemIcon::Native(parse_native_icon(name)?)),
        (None, None) => Ok(ItemIcon::None),
    }
}

/// Recursively fill `container`, registering every addressable item through
/// `register`. Used by the menu bar and the tray (an `id -> handle` map for
/// the runtime mutation APIs) and by context menus (to know which ids a
/// popup contains).
///
/// Items without an `id` (separators, label-less roles) are still appended
/// — they're just not registered: only addressable items participate in the
/// dynamic API.
pub(crate) fn fill_with_registry<F>(
    container: &RegContainer,
    items: &[MenuItemDef],
    register: &mut F,
) -> Result<(), String>
where
    F: FnMut(String, RegEntry),
{
    for item in items {
        insert_with_registry(container, item, None, register)?;
    }
    Ok(())
}

/// Build one item (and its subtree) and append it to `container`, or insert
/// it at index `at`.
pub(crate) fn insert_with_registry<F>(
    container: &RegContainer,
    item: &MenuItemDef,
    at: Option<usize>,
    register: &mut F,
) -> Result<(), String>
where
    F: FnMut(String, RegEntry),
{
    let slot = Slot {
        container,
        at,
        position: at.unwrap_or_else(|| container.len()),
    };
    match item {
        MenuItemDef::Separator => container.place(&muda::PredefinedMenuItem::separator(), at),
        MenuItemDef::Submenu(submenu) => append_registered_submenu(&slot, submenu, register),
        MenuItemDef::Checkbox(CheckboxDef {
            id,
            label,
            enabled,
            checked,
            accelerator,
        }) => append_registered_check(
            &slot,
            CheckFields {
                id: id.as_deref(),
                label: label.as_deref(),
                enabled: *enabled,
                checked: *checked,
                accelerator: accelerator.as_deref(),
                radio: false,
            },
            register,
        ),
        MenuItemDef::Radio(RadioDef {
            id,
            label,
            enabled,
            checked,
            accelerator,
        }) => append_registered_check(
            &slot,
            CheckFields {
                id: id.as_deref(),
                label: label.as_deref(),
                enabled: *enabled,
                checked: *checked,
                accelerator: accelerator.as_deref(),
                radio: true,
            },
            register,
        ),
        MenuItemDef::Item(item) => append_registered_regular_item(&slot, item, register),
    }
}

/// Where an item goes: its container, the requested insert index (`None` =
/// append) and the resulting index recorded in the registry.
struct Slot<'a> {
    container: &'a RegContainer,
    at: Option<usize>,
    position: usize,
}

impl Slot<'_> {
    fn entry(&self, item: RegItem, radio: bool, accelerator: Option<&str>) -> RegEntry {
        RegEntry {
            item,
            parent: self.container.clone(),
            position: self.position,
            radio,
            accelerator: accelerator.map(str::to_string),
        }
    }
}

/// Shared fields of checkbox and radio items.
struct CheckFields<'a> {
    id: Option<&'a str>,
    label: Option<&'a str>,
    enabled: Option<bool>,
    checked: Option<bool>,
    accelerator: Option<&'a str>,
    radio: bool,
}

fn append_registered_check<F>(
    slot: &Slot<'_>,
    fields: CheckFields<'_>,
    register: &mut F,
) -> Result<(), String>
where
    F: FnMut(String, RegEntry),
{
    let Some(label) = fields.label else {
        return Ok(());
    };
    let id_value = fields.id.unwrap_or(label);
    let item = muda::CheckMenuItem::with_id(
        muda::MenuId::new(id_value),
        label,
        fields.enabled.unwrap_or(true),
        fields.checked.unwrap_or(false),
        parse_accel_str(fields.accelerator),
    );
    slot.container.place(&item, slot.at)?;
    register(
        id_value.to_string(),
        slot.entry(RegItem::Check(item), fields.radio, fields.accelerator),
    );
    Ok(())
}

fn append_registered_submenu<F>(
    slot: &Slot<'_>,
    submenu: &SubmenuDef,
    register: &mut F,
) -> Result<(), String>
where
    F: FnMut(String, RegEntry),
{
    let label = submenu.label.as_deref().unwrap_or("");
    let enabled = submenu.enabled.unwrap_or(true);
    let id_value = submenu.id.as_deref().unwrap_or(label);
    let icon = resolve_icon(submenu.icon.as_ref(), submenu.native_icon.as_deref())?;
    let item = muda::Submenu::with_id(muda::MenuId::new(id_value), label, enabled);
    match icon {
        ItemIcon::None => {},
        ItemIcon::Custom(icon) => item.set_icon(Some(icon)),
        ItemIcon::Native(icon) => item.set_native_icon(Some(icon)),
    }
    fill_with_registry(&RegContainer::Sub(item.clone()), &submenu.items, register)?;
    slot.container.place(&item, slot.at)?;
    if let Some(id) = submenu.id.as_deref() {
        register(
            id.to_string(),
            slot.entry(RegItem::Submenu(item), false, None),
        );
    }
    Ok(())
}

fn append_registered_regular_item<F>(
    slot: &Slot<'_>,
    item: &ItemDef,
    register: &mut F,
) -> Result<(), String>
where
    F: FnMut(String, RegEntry),
{
    if let Some(role) = item.role {
        let predefined = role_to_predefined(role, item.label.as_deref());
        slot.container.place(&predefined, slot.at)?;
        if let Some(id) = item.id.as_deref() {
            register(
                id.to_string(),
                slot.entry(RegItem::Predefined(predefined), false, None),
            );
        }
        return Ok(());
    }

    let Some(label) = &item.label else {
        return Ok(());
    };
    let id_value = item.id.as_deref().unwrap_or(label);
    let id = muda::MenuId::new(id_value);
    let enabled = item.enabled.unwrap_or(true);
    let accelerator = parse_accel_str(item.accelerator.as_deref());
    let reg_item = match resolve_icon(item.icon.as_ref(), item.native_icon.as_deref())? {
        ItemIcon::None => {
            RegItem::MenuItem(muda::MenuItem::with_id(id, label, enabled, accelerator))
        },
        ItemIcon::Custom(icon) => RegItem::Icon(muda::IconMenuItem::with_id(
            id,
            label,
            enabled,
            Some(icon),
            accelerator,
        )),
        ItemIcon::Native(icon) => RegItem::Icon(muda::IconMenuItem::with_id_and_native_icon(
            id,
            label,
            enabled,
            Some(icon),
            accelerator,
        )),
    };
    slot.container.place(reg_item.as_is_menu_item(), slot.at)?;
    register(
        id_value.to_string(),
        slot.entry(reg_item, false, item.accelerator.as_deref()),
    );
    Ok(())
}

/// Native predefined item for `role`; `text` overrides the OS label where
/// the platform allows it. Per-OS availability follows muda: `fullscreen`,
/// `hideOthers`, `showAll`, `services` and `bringAllToFront` are macOS-only;
/// Linux (GTK 3) only has the edit roles (`undo` … `selectAll`) and
/// `separator`. An unsupported role renders nothing on that OS.
fn role_to_predefined(role: MenuRole, text: Option<&str>) -> muda::PredefinedMenuItem {
    use muda::PredefinedMenuItem as P;
    match role {
        MenuRole::Separator => P::separator(),
        MenuRole::Quit => P::quit(text),
        MenuRole::Copy => P::copy(text),
        MenuRole::Cut => P::cut(text),
        MenuRole::Paste => P::paste(text),
        MenuRole::Undo => P::undo(text),
        MenuRole::Redo => P::redo(text),
        MenuRole::SelectAll => P::select_all(text),
        MenuRole::Minimize => P::minimize(text),
        MenuRole::Maximize => P::maximize(text),
        MenuRole::Fullscreen => P::fullscreen(text),
        MenuRole::Close => P::close_window(text),
        MenuRole::About => P::about(text, None),
        MenuRole::Hide => P::hide(text),
        MenuRole::HideOthers => P::hide_others(text),
        MenuRole::ShowAll => P::show_all(text),
        MenuRole::Services => P::services(text),
        MenuRole::BringAllToFront => P::bring_all_to_front(text),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rgba_icon_validates_size_and_length() {
        let pixels = STANDARD.encode([0_u8; 16]);
        let ok = MenuIcon::Rgba {
            rgba: pixels.clone(),
            width: 2,
            height: 2,
        };
        assert_eq!(icon_rgba(&ok).map(|(_, w, h)| (w, h)), Ok((2, 2)));

        let short = MenuIcon::Rgba {
            rgba: pixels.clone(),
            width: 4,
            height: 4,
        };
        assert!(icon_rgba(&short)
            .unwrap_err()
            .contains("expected width * height * 4"));

        let huge = MenuIcon::Rgba {
            rgba: pixels,
            width: 4096,
            height: 1,
        };
        assert!(icon_rgba(&huge).unwrap_err().contains("pixels"));
    }

    #[test]
    fn base64_icons_reject_garbage() {
        let bad = MenuIcon::Png { png: "%%%".into() };
        assert!(icon_rgba(&bad).unwrap_err().contains("base64"));
        let not_png = MenuIcon::Png {
            png: STANDARD.encode(b"hello"),
        };
        assert!(icon_rgba(&not_png).unwrap_err().contains("PNG"));
    }

    #[test]
    fn png_icon_decodes() {
        let mut bytes = std::io::Cursor::new(Vec::new());
        image::RgbaImage::new(3, 2)
            .write_to(&mut bytes, image::ImageFormat::Png)
            .unwrap();
        let icon = MenuIcon::Png {
            png: STANDARD.encode(bytes.into_inner()),
        };
        let (rgba, width, height) = icon_rgba(&icon).unwrap();
        assert_eq!((rgba.len(), width, height), (24, 3, 2));
    }

    #[test]
    fn native_icon_names() {
        assert!(matches!(
            parse_native_icon("add"),
            Ok(muda::NativeIcon::Add)
        ));
        assert!(matches!(
            parse_native_icon("trashFull"),
            Ok(muda::NativeIcon::TrashFull)
        ));
        assert!(parse_native_icon("Add")
            .unwrap_err()
            .contains("unknown native icon"));
    }

    #[test]
    fn icon_and_native_icon_are_exclusive() {
        let icon = MenuIcon::Path("x.png".into());
        let error = resolve_icon(Some(&icon), Some("add")).err().unwrap();
        assert!(error.contains("not both"));
        assert!(matches!(resolve_icon(None, None), Ok(ItemIcon::None)));
    }
}
