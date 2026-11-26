//! System tray icons (`tray.*`): any number of icons keyed by id, each with
//! its own menu, events and optional attached window. The `tray` entry of
//! `app.start` becomes the icon with id [`MAIN_TRAY`].
//!
//! Every icon lives on the main thread (muda / tray-icon handles are `!Send`):
//! renderer calls arrive through `UserEvent::TrayCmd`, icon clicks that toggle
//! an attached window through `UserEvent::TrayToggleWindow`.

mod attach;
mod geometry;
mod items;

use std::{
    cell::RefCell,
    collections::BTreeMap,
    sync::{Mutex, OnceLock},
};

use serde_json::{json, Map, Value};
use tao::{event_loop::EventLoopProxy, window::Window};
use tray_icon::{MouseButton, MouseButtonState, TrayIconEvent};

use tynd_protocol::runtime::{MenuIcon, MenuItemDef, TrayConfig};

use self::{
    attach::Attachment,
    geometry::Bounds,
    items::{optional_field, TrayMenu},
};
use crate::{
    api::menu::builder::{icon_rgba, parse_native_icon},
    app::UserEvent,
    ipc::error::IpcError,
};

/// Id of the icon declared by `app.start({ tray })`, and the default target
/// of every call that names no `tray`.
pub(crate) const MAIN_TRAY: &str = "main";

/// Upper bound on live icons: a renderer cannot flood the notification area.
const MAX_TRAYS: usize = 16;

struct TrayInstance {
    icon: tray_icon::TrayIcon,
    menu: TrayMenu,
    attachment: Option<Attachment>,
}

thread_local! {
    /// Live icons by id (main thread). Ordered so `list` is stable.
    static TRAYS: RefCell<BTreeMap<String, TrayInstance>> = const { RefCell::new(BTreeMap::new()) };
}

/// Proxy used by the tray event handler (any thread on Linux) to reach the
/// event loop when a click must toggle an attached window.
static PROXY: OnceLock<Mutex<EventLoopProxy<UserEvent>>> = OnceLock::new();

/// Handle held by the event loop for the lifetime of the app. Dropping it
/// removes every icon from the notification area.
pub(crate) struct SystemTray(());

impl Drop for SystemTray {
    fn drop(&mut self) {
        let _ = TRAYS.try_with(|trays| {
            if let Ok(mut trays) = trays.try_borrow_mut() {
                trays.clear();
            }
        });
    }
}

/// Error returned when a renderer calls `tray.*` and no tray is available.
pub(crate) fn unavailable_error() -> String {
    "tray: the tray API is not initialised".into()
}

/// Install the event handler and build the configured `main` icon. Always
/// returns the handle: icons can still be created at runtime when the
/// configured one fails.
pub(crate) fn init(config: Option<&TrayConfig>, proxy: &EventLoopProxy<UserEvent>) -> SystemTray {
    let _ = PROXY.set(Mutex::new(proxy.clone()));
    TrayIconEvent::set_event_handler(Some(handle_event));
    if let Some(config) = config {
        let options = TrayOptions {
            icon: Some(TrayImage::Path(config.icon.clone())),
            tooltip: config.tooltip.clone(),
            menu: config.menu.clone().unwrap_or_default(),
            ..TrayOptions::default()
        };
        if let Err(error) = create(MAIN_TRAY, options) {
            crate::tynd_warn!("System tray failed: {error}");
        }
    }
    SystemTray(())
}

/// Windows a tray call can reach: the caller's, or any open one by label.
pub(crate) struct TrayCall<'a> {
    pub(crate) caller: &'a str,
    /// Effective origin of the calling frame; `None` for app-local content.
    pub(crate) caller_origin: Option<&'a str>,
    pub(crate) window_for: &'a dyn Fn(&str) -> Option<&'a Window>,
}

impl TrayCall<'_> {
    /// Remote content may only attach or position the window it runs in,
    /// never hide or move another window by label.
    fn check_target(&self, op: &str, label: &str) -> Result<(), String> {
        if self.caller_origin.is_some() && label != self.caller {
            return Err(format!(
                "tray.{op}: remote content can only drive its own window"
            ));
        }
        Ok(())
    }
}

/// Renderer `tray.*` calls, on the main thread.
pub(crate) fn dispatch(
    _tray: &SystemTray,
    call: &TrayCall<'_>,
    method: &str,
    args: &Value,
) -> Result<Value, String> {
    match method {
        "create" => {
            let id = tray_id(args)?;
            create(&id, TrayOptions::parse(args)?)?;
            Ok(Value::Null)
        },
        "list" => Ok(TRAYS.with(|trays| json!(trays.borrow().keys().collect::<Vec<_>>()))),
        "remove" => {
            let id = tray_id(args)?;
            TRAYS
                .with(|trays| trays.borrow_mut().remove(&id))
                .map(|_| Value::Null)
                .ok_or_else(|| missing_tray(&id))
        },
        "popUpContextMenu" => {
            // Modal on Windows / macOS: a click inside must find the registry
            // unborrowed (radio groups, `menu:action`'s `tray` field).
            let id = tray_id(args)?;
            let icon = with_tray(&id, |tray| Ok(tray.icon.clone()))?;
            pop_up_menu(&icon)?;
            Ok(Value::Null)
        },
        _ => {
            let id = tray_id(args)?;
            with_tray(&id, |tray| dispatch_on(tray, call, method, args))
        },
    }
}

fn dispatch_on(
    tray: &mut TrayInstance,
    call: &TrayCall<'_>,
    method: &str,
    args: &Value,
) -> Result<Value, String> {
    if let Some(result) = tray.menu.dispatch(method, args) {
        return result.map(|()| Value::Null);
    }
    match method {
        "setIcon" => {
            let image = TrayImage::parse(args, "setIcon")?
                .ok_or_else(|| "tray.setIcon: pass 'icon' or 'nativeIcon'".to_string())?;
            let template = optional_bool(args, "setIcon", "template")?.unwrap_or(false);
            apply_icon(&tray.icon, image, template)?;
        },
        "setIconAsTemplate" => set_icon_as_template(
            &tray.icon,
            required_bool(args, "setIconAsTemplate", "template")?,
        ),
        "setTooltip" => {
            let text = optional_string(args, "setTooltip", "text")?;
            tray.icon
                .set_tooltip(text)
                .map_err(|error| format!("tray.setTooltip: {error}"))?;
        },
        "setTitle" => tray
            .icon
            .set_title(optional_string(args, "setTitle", "text")?),
        "setVisible" => tray
            .icon
            .set_visible(required_bool(args, "setVisible", "visible")?)
            .map_err(|error| format!("tray.setVisible: {error}"))?,
        "setShowMenuOnLeftClick" => tray.icon.set_show_menu_on_left_click(required_bool(
            args,
            "setShowMenuOnLeftClick",
            "show",
        )?),
        "setMenu" => {
            let items: Vec<MenuItemDef> = optional_field(args, "setMenu", "items")?
                .ok_or_else(|| "tray.setMenu: 'items' must be an array".to_string())?;
            let menu = items::build(&items).map_err(|error| format!("tray.setMenu: {error}"))?;
            tray.icon.set_menu(Some(Box::new(menu.root().clone())));
            tray.menu = menu;
        },
        "getRect" => return Ok(tray.icon.rect().map_or(Value::Null, rect_json)),
        "attachWindow" => {
            attach::check_supported()?;
            let attachment = Attachment::parse(args)?;
            call.check_target("attachWindow", &attachment.label)?;
            let window = (call.window_for)(&attachment.label).ok_or_else(|| {
                format!(
                    "tray.attachWindow: no open window '{}' — create it first",
                    attachment.label
                )
            })?;
            window.set_visible(false);
            // The left click now toggles the window instead of the menu.
            tray.icon.set_show_menu_on_left_click(false);
            tray.attachment = Some(attachment);
        },
        "detachWindow" => {
            tray.attachment = None;
            tray.icon.set_show_menu_on_left_click(true);
        },
        "positionWindow" => {
            let label = optional_string(args, "positionWindow", "label")?
                .unwrap_or_else(|| call.caller.to_string());
            call.check_target("positionWindow", &label)?;
            let gap = attach::parse_gap(args, "positionWindow")?;
            let window = (call.window_for)(&label)
                .ok_or_else(|| format!("tray.positionWindow: no open window '{label}'"))?;
            let anchor = if let Some(bounds) = tray.icon.rect().and_then(rect_bounds) {
                bounds
            } else {
                // No icon bounds (Linux): open next to the pointer.
                let cursor = window
                    .cursor_position()
                    .map_err(|error| format!("tray.positionWindow: {error}"))?;
                point_bounds(cursor.x, cursor.y)
            };
            attach::place(window, &anchor, gap);
        },
        _ => return Err(IpcError::unknown_method("tray", method).into_string()),
    }
    Ok(Value::Null)
}

/// Icon source of `create` / `setIcon`.
enum TrayImage {
    /// Config icon: a file path of any size (fs allow-list applies).
    Path(String),
    Rgba(Vec<u8>, u32, u32),
    Native(tray_icon::NativeIcon),
}

impl TrayImage {
    /// `icon` (path, `{ png }` or `{ rgba, width, height }`) or `nativeIcon`.
    fn parse(args: &Value, op: &str) -> Result<Option<Self>, String> {
        let icon = optional_field::<MenuIcon>(args, op, "icon")?;
        let native = optional_field::<String>(args, op, "nativeIcon")?;
        match (icon, native) {
            (Some(_), Some(_)) => Err(format!(
                "tray.{op}: set either 'icon' or 'nativeIcon', not both"
            )),
            (Some(MenuIcon::Path(path)), None) => Ok(Some(Self::Path(path))),
            (Some(icon), None) => {
                let (rgba, width, height) =
                    icon_rgba(&icon).map_err(|error| format!("tray.{op}: {error}"))?;
                Ok(Some(Self::Rgba(rgba, width, height)))
            },
            (None, Some(name)) => parse_native_icon(&name)
                .map(|icon| Some(Self::Native(icon)))
                .map_err(|error| format!("tray.{op}: {error}")),
            (None, None) => Ok(None),
        }
    }

    fn custom(self) -> Result<Result<tray_icon::Icon, tray_icon::NativeIcon>, String> {
        match self {
            Self::Path(path) => crate::platform::icon::load_tray(&path).map(Ok),
            Self::Rgba(rgba, width, height) => tray_icon::Icon::from_rgba(rgba, width, height)
                .map(Ok)
                .map_err(|error| error.to_string()),
            Self::Native(icon) => Ok(Err(icon)),
        }
    }
}

/// Options of `create` (and of the configured `main` icon).
#[derive(Default)]
struct TrayOptions {
    icon: Option<TrayImage>,
    template: bool,
    tooltip: Option<String>,
    title: Option<String>,
    menu: Vec<MenuItemDef>,
    show_menu_on_left_click: Option<bool>,
    visible: Option<bool>,
}

impl TrayOptions {
    fn parse(args: &Value) -> Result<Self, String> {
        let icon = TrayImage::parse(args, "create")?;
        if icon.is_none() {
            return Err(
                "tray.create: pass 'icon' (path, { png } or { rgba, width, height }) or 'nativeIcon'"
                    .into(),
            );
        }
        Ok(Self {
            icon,
            template: optional_bool(args, "create", "template")?.unwrap_or(false),
            tooltip: optional_string(args, "create", "tooltip")?,
            title: optional_string(args, "create", "title")?,
            menu: optional_field(args, "create", "menu")?.unwrap_or_default(),
            show_menu_on_left_click: optional_bool(args, "create", "showMenuOnLeftClick")?,
            visible: optional_bool(args, "create", "visible")?,
        })
    }
}

fn create(id: &str, options: TrayOptions) -> Result<(), String> {
    TRAYS.with(|trays| {
        let trays = trays.borrow();
        if trays.contains_key(id) {
            return Err(format!(
                "tray.create: a tray with id '{id}' already exists — call tray.remove(\"{id}\") first"
            ));
        }
        if trays.len() >= MAX_TRAYS {
            return Err(format!("tray.create: at most {MAX_TRAYS} trays can exist"));
        }
        Ok(())
    })?;
    let menu = items::build(&options.menu).map_err(|error| format!("tray.create: {error}"))?;
    let mut builder = tray_icon::TrayIconBuilder::new()
        .with_id(id)
        .with_menu(Box::new(menu.root().clone()));
    if let Some(tooltip) = &options.tooltip {
        builder = builder.with_tooltip(tooltip);
    }
    if let Some(title) = &options.title {
        builder = builder.with_title(title);
    }
    if let Some(show) = options.show_menu_on_left_click {
        builder = builder.with_menu_on_left_click(show);
    }
    let icon = builder
        .build()
        .map_err(|error| format!("tray.create: {error}"))?;
    if let Some(image) = options.icon {
        apply_icon(&icon, image, options.template)?;
    }
    if options.visible == Some(false) {
        icon.set_visible(false)
            .map_err(|error| format!("tray.create: {error}"))?;
    }
    TRAYS.with(|trays| {
        trays.borrow_mut().insert(
            id.to_string(),
            TrayInstance {
                icon,
                menu,
                attachment: None,
            },
        );
    });
    Ok(())
}

fn apply_icon(icon: &tray_icon::TrayIcon, image: TrayImage, template: bool) -> Result<(), String> {
    match image.custom()? {
        Ok(custom) => set_custom_icon(icon, custom, template),
        Err(native) => icon.set_native_icon(Some(native)),
    }
    .map_err(|error| format!("tray icon: {error}"))
}

/// macOS draws a template icon from its alpha channel only, so it follows the
/// light / dark menu bar. Ignored elsewhere.
#[cfg(target_os = "macos")]
fn set_custom_icon(
    icon: &tray_icon::TrayIcon,
    custom: tray_icon::Icon,
    template: bool,
) -> tray_icon::Result<()> {
    if template {
        icon.set_icon_templated(Some(custom))
    } else {
        icon.set_icon(Some(custom))
    }
}

#[cfg(not(target_os = "macos"))]
fn set_custom_icon(
    icon: &tray_icon::TrayIcon,
    custom: tray_icon::Icon,
    _template: bool,
) -> tray_icon::Result<()> {
    icon.set_icon(Some(custom))
}

// tray-icon 0.26 deprecates this toggle in favour of `set_icon_templated`,
// which needs the icon re-supplied; the toggle keeps working on the current
// icon (no-op off macOS).
#[allow(deprecated)]
fn set_icon_as_template(icon: &tray_icon::TrayIcon, template: bool) {
    icon.set_icon_as_template(template);
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
#[allow(clippy::unnecessary_wraps)]
fn pop_up_menu(icon: &tray_icon::TrayIcon) -> Result<(), String> {
    icon.show_menu();
    Ok(())
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn pop_up_menu(_icon: &tray_icon::TrayIcon) -> Result<(), String> {
    Err("tray.popUpContextMenu: not supported on linux (the panel opens the menu itself)".into())
}

/// Tray id of a call: `tray` (default `"main"`), 1-64 letters, digits, `_`, `-`.
fn tray_id(args: &Value) -> Result<String, String> {
    let id = match args.get("tray") {
        None | Some(Value::Null) => return Ok(MAIN_TRAY.into()),
        Some(Value::String(id)) => id,
        Some(_) => return Err("tray: 'tray' must be a string id".into()),
    };
    let valid = !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if valid {
        Ok(id.clone())
    } else {
        Err(format!(
            "tray: invalid tray id {id:?} (1-64 letters, digits, '_' or '-')"
        ))
    }
}

fn missing_tray(id: &str) -> String {
    if id == MAIN_TRAY {
        "tray: no \"main\" tray — set `tray` in app.start or call tray.create(\"main\", options)"
            .into()
    } else {
        format!("tray: no tray with id '{id}' — call tray.create(\"{id}\", options) first")
    }
}

fn with_tray<T>(
    id: &str,
    f: impl FnOnce(&mut TrayInstance) -> Result<T, String>,
) -> Result<T, String> {
    TRAYS.with(|trays| {
        let mut trays = trays.borrow_mut();
        let tray = trays.get_mut(id).ok_or_else(|| missing_tray(id))?;
        f(tray)
    })
}

fn optional_bool(args: &Value, op: &str, field: &str) -> Result<Option<bool>, String> {
    optional_field(args, op, field)
}

fn required_bool(args: &Value, op: &str, field: &str) -> Result<bool, String> {
    optional_bool(args, op, field)?.ok_or_else(|| format!("tray.{op}: missing '{field}'"))
}

fn optional_string(args: &Value, op: &str, field: &str) -> Result<Option<String>, String> {
    optional_field(args, op, field)
}

fn rect_bounds(rect: tray_icon::Rect) -> Option<Bounds> {
    // Linux reports an empty rect: no icon bounds available.
    (rect.size.width > 0 && rect.size.height > 0).then(|| Bounds {
        x: rect.position.x,
        y: rect.position.y,
        width: f64::from(rect.size.width),
        height: f64::from(rect.size.height),
    })
}

fn point_bounds(x: f64, y: f64) -> Bounds {
    Bounds {
        x,
        y,
        width: 0.0,
        height: 0.0,
    }
}

fn rect_json(rect: tray_icon::Rect) -> Value {
    rect_bounds(rect).map_or(Value::Null, |bounds| {
        json!({ "x": bounds.x, "y": bounds.y, "width": bounds.width, "height": bounds.height })
    })
}

fn button_name(button: MouseButton) -> &'static str {
    match button {
        MouseButton::Left => "left",
        MouseButton::Right => "right",
        MouseButton::Middle => "middle",
    }
}

/// Event name and payload for a tray event. A click counts on release only:
/// the press would fire `tray:click` a second time.
fn event_payload(event: &TrayIconEvent) -> Option<(&'static str, Value)> {
    let (name, id, position, rect, button) = match event {
        TrayIconEvent::Click {
            id,
            position,
            rect,
            button,
            button_state,
        } => {
            if *button_state != MouseButtonState::Up {
                return None;
            }
            let name = match button {
                MouseButton::Left => "tray:click",
                MouseButton::Right => "tray:right-click",
                MouseButton::Middle => "tray:middle-click",
            };
            (name, id, position, rect, Some(*button))
        },
        TrayIconEvent::DoubleClick {
            id,
            position,
            rect,
            button,
        } => ("tray:double-click", id, position, rect, Some(*button)),
        TrayIconEvent::Enter { id, position, rect } => ("tray:enter", id, position, rect, None),
        TrayIconEvent::Move { id, position, rect } => ("tray:move", id, position, rect, None),
        TrayIconEvent::Leave { id, position, rect } => ("tray:leave", id, position, rect, None),
        _ => return None,
    };
    let mut payload = Map::new();
    payload.insert("tray".into(), json!(id.0));
    if let Some(button) = button {
        payload.insert("button".into(), json!(button_name(button)));
    }
    payload.insert(
        "position".into(),
        json!({ "x": position.x, "y": position.y }),
    );
    payload.insert("rect".into(), rect_json(*rect));
    Some((name, Value::Object(payload)))
}

/// tray-icon event handler (main thread on Windows / macOS, a D-Bus thread on
/// some Linux desktops): forward to the renderer, and let the event loop
/// toggle the attached window on a left click.
fn handle_event(event: TrayIconEvent) {
    let Some((name, payload)) = event_payload(&event) else {
        return;
    };
    if let TrayIconEvent::Click {
        id, position, rect, ..
    } = &event
    {
        if name == "tray:click" {
            let anchor = rect_bounds(*rect).unwrap_or_else(|| point_bounds(position.x, position.y));
            request_toggle(&id.0, &anchor);
        }
    }
    crate::ipc::events::emit(name, &payload);
}

fn request_toggle(tray: &str, anchor: &Bounds) {
    let Some(proxy) = PROXY.get() else {
        return;
    };
    let proxy = proxy
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let _ = proxy.send_event(UserEvent::TrayToggleWindow {
        tray: tray.to_string(),
        anchor: [anchor.x, anchor.y, anchor.width, anchor.height],
    });
}

/// `UserEvent::TrayToggleWindow`: toggle the window attached to `tray`, if any.
pub(crate) fn toggle_attached<'w>(
    tray: &str,
    anchor: [f64; 4],
    window_for: &dyn Fn(&str) -> Option<&'w Window>,
) {
    let attachment = TRAYS
        .try_with(|trays| {
            trays
                .try_borrow()
                .ok()
                .and_then(|trays| trays.get(tray).and_then(|tray| tray.attachment.clone()))
        })
        .ok()
        .flatten();
    let Some(attachment) = attachment else {
        return;
    };
    let Some(window) = window_for(&attachment.label) else {
        crate::tynd_warn!(
            "tray '{tray}': attached window '{}' is closed",
            attachment.label
        );
        return;
    };
    let [x, y, width, height] = anchor;
    attach::toggle(
        &attachment,
        window,
        &Bounds {
            x,
            y,
            width,
            height,
        },
    );
}

/// Window focus lost: hide it when a tray attached it with `hideOnBlur`.
pub(crate) fn on_window_blurred(label: &str, window: Option<&Window>) {
    let Some(window) = window else {
        return;
    };
    let attachment = TRAYS
        .try_with(|trays| {
            trays.try_borrow().ok().and_then(|trays| {
                trays
                    .values()
                    .filter_map(|tray| tray.attachment.as_ref())
                    .find(|attachment| attachment.label == label)
                    .cloned()
            })
        })
        .ok()
        .flatten();
    if let Some(attachment) = attachment {
        attach::hide_on_blur(&attachment, window);
    }
}

/// Menu click hook: keeps tray radio groups exclusive and returns the ids of
/// the trays whose menu claims the click on `item` (several: an ambiguous
/// plain id).
pub(crate) fn on_menu_activated(item: &str) -> Vec<String> {
    TRAYS
        .try_with(|trays| {
            let Ok(trays) = trays.try_borrow() else {
                return Vec::new();
            };
            trays
                .iter()
                .filter(|(_, tray)| tray.menu.on_activated(item))
                .map(|(id, _)| id.clone())
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use tray_icon::{
        dpi::{PhysicalPosition, PhysicalSize},
        Rect, TrayIconId,
    };

    use super::*;

    #[test]
    fn remote_callers_only_attach_or_position_their_own_window() {
        let remote = TrayCall {
            caller: "panel",
            caller_origin: Some("https://remote.example"),
            window_for: &|_| None,
        };
        assert!(remote.check_target("attachWindow", "panel").is_ok());
        let error = remote
            .check_target("attachWindow", "main")
            .expect_err("another window");
        assert!(error.contains("remote content"), "{error}");
        assert!(remote.check_target("positionWindow", "settings").is_err());
        let local = TrayCall {
            caller: "panel",
            caller_origin: None,
            window_for: &|_| None,
        };
        assert!(local.check_target("attachWindow", "main").is_ok());
    }

    fn click(button: MouseButton, button_state: MouseButtonState, rect: Rect) -> TrayIconEvent {
        TrayIconEvent::Click {
            id: TrayIconId::new("status"),
            position: PhysicalPosition::new(10.0, 20.0),
            rect,
            button,
            button_state,
        }
    }

    fn icon_rect() -> Rect {
        Rect {
            size: PhysicalSize::new(24, 32),
            position: PhysicalPosition::new(1700.0, 1040.0),
        }
    }

    #[test]
    fn clicks_fire_once_on_release_with_button_position_and_rect() {
        assert!(event_payload(&click(
            MouseButton::Left,
            MouseButtonState::Down,
            icon_rect()
        ))
        .is_none());
        let (name, payload) =
            event_payload(&click(MouseButton::Left, MouseButtonState::Up, icon_rect())).unwrap();
        assert_eq!(name, "tray:click");
        assert_eq!(
            payload,
            json!({
                "tray": "status",
                "button": "left",
                "position": { "x": 10.0, "y": 20.0 },
                "rect": { "x": 1700.0, "y": 1040.0, "width": 24.0, "height": 32.0 }
            })
        );
        let names: Vec<_> = [MouseButton::Right, MouseButton::Middle]
            .into_iter()
            .map(|button| {
                event_payload(&click(button, MouseButtonState::Up, icon_rect()))
                    .unwrap()
                    .0
            })
            .collect();
        assert_eq!(names, ["tray:right-click", "tray:middle-click"]);
    }

    #[test]
    fn empty_rect_is_reported_as_null() {
        let (_, payload) = event_payload(&TrayIconEvent::Enter {
            id: TrayIconId::new("main"),
            position: PhysicalPosition::new(1.0, 2.0),
            rect: Rect::default(),
        })
        .unwrap();
        assert_eq!(payload["rect"], Value::Null);
        assert!(payload.get("button").is_none());
    }

    #[test]
    fn tray_ids_default_to_main_and_are_validated() {
        assert_eq!(tray_id(&json!({})).unwrap(), MAIN_TRAY);
        assert_eq!(tray_id(&json!({ "tray": "status-2" })).unwrap(), "status-2");
        for bad in [
            json!(""),
            json!("a b"),
            json!("../x"),
            json!(3),
            json!("x".repeat(65)),
        ] {
            assert!(tray_id(&json!({ "tray": bad })).is_err(), "{bad}");
        }
    }

    #[test]
    fn create_options_require_one_icon_source() {
        let missing = TrayOptions::parse(&json!({ "tooltip": "t" }))
            .err()
            .unwrap();
        assert!(missing.contains("pass 'icon'"), "{missing}");
        let both = TrayOptions::parse(&json!({ "icon": "a.png", "nativeIcon": "add" }))
            .err()
            .unwrap();
        assert!(both.contains("not both"), "{both}");
        let bad_native = TrayOptions::parse(&json!({ "nativeIcon": "nope" }))
            .err()
            .unwrap();
        assert!(bad_native.contains("unknown native icon"), "{bad_native}");
        let bad_rgba =
            TrayOptions::parse(&json!({ "icon": { "rgba": "AAAA", "width": 2, "height": 2 } }))
                .err()
                .unwrap();
        assert!(
            bad_rgba.contains("expected width * height * 4"),
            "{bad_rgba}"
        );

        let options = TrayOptions::parse(&json!({
            "icon": { "rgba": "AAAAAA==", "width": 1, "height": 1 },
            "template": true,
            "title": "42",
            "showMenuOnLeftClick": false,
            "menu": [{ "id": "quit", "label": "Quit" }]
        }))
        .unwrap();
        assert!(matches!(options.icon, Some(TrayImage::Rgba(_, 1, 1))));
        assert!(options.template);
        assert_eq!(options.title.as_deref(), Some("42"));
        assert_eq!(options.show_menu_on_left_click, Some(false));
        assert_eq!(options.menu.len(), 1);

        let wrong_type = TrayOptions::parse(&json!({ "nativeIcon": "add", "visible": "no" }))
            .err()
            .unwrap();
        assert!(wrong_type.contains("invalid 'visible'"), "{wrong_type}");
    }

    #[test]
    fn missing_tray_errors_say_what_to_do() {
        assert!(missing_tray(MAIN_TRAY).contains("set `tray` in app.start"));
        assert!(missing_tray("status").contains("tray.create(\"status\""));
    }
}
