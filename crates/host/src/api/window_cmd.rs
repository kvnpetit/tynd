use std::sync::OnceLock;

use dpi::{LogicalPosition, LogicalSize, PhysicalPosition};
use serde_json::{json, Value};
use tao::window::{CursorIcon, ProgressBarState, ProgressState, ResizeDirection, Theme, Window};

mod drag_out;
mod frame;
mod monitor;
mod platform;

use frame::dispatch_frame;
pub(crate) use frame::set_kiosk;
#[cfg(feature = "api-tray")]
pub(crate) use monitor::work_area as monitor_work_area;
pub(crate) use monitor::{refresh as refresh_monitors, set_change_hook as set_monitor_change_hook};
use platform::{dispatch_platform, set_enabled};

// `center` calls `crate::window::center_window` (the host window layer)
// directly: both live in the same crate, so no startup-registered hook is
// needed.

type BackendWindowHook = Box<dyn Fn(&str, Value) -> Result<Value, String> + Send + Sync>;

/// Installed by the event loop at startup: posts a backend window command to
/// the main thread and waits for its answer.
static BACKEND_WINDOW_HOOK: OnceLock<BackendWindowHook> = OnceLock::new();

pub(crate) fn set_backend_window_hook(hook: BackendWindowHook) {
    if BACKEND_WINDOW_HOOK.set(hook).is_err() {
        crate::tynd_warn!("backend window hook already installed");
    }
}

/// Run a `window.*` command for the trusted backend (`hostCall("window", …)`
/// from Bun). `args.window` names the target window (default `"main"`).
/// Blocks the calling thread until the event loop answers, so never call it
/// from the main thread.
///
/// # Errors
/// The command's own error, or an error when the event loop is not running.
pub fn dispatch_backend(method: &str, args: Value) -> Result<Value, String> {
    let hook = BACKEND_WINDOW_HOOK
        .get()
        .ok_or_else(|| format!("window.{method}: the window event loop is not running"))?;
    hook(method, args)
}

/// Dispatch a window command synchronously on the main (event-loop) thread.
pub(crate) fn dispatch(win: &Window, method: &str, args: &Value) -> Result<Value, String> {
    if let Some(result) = dispatch_geometry(win, method, args) {
        return result;
    }
    if let Some(result) = dispatch_window_configuration(win, method, args) {
        return result;
    }
    if let Some(result) = dispatch_window_action(win, method) {
        return result;
    }
    if let Some(result) = monitor::dispatch(win, method, args) {
        return result;
    }
    if let Some(result) = dispatch_pointer(win, method, args) {
        return result;
    }
    if let Some(result) = dispatch_appearance(win, method, args) {
        return result;
    }
    if let Some(result) = dispatch_frame(win, method, args) {
        return result;
    }
    if let Some(result) = dispatch_platform(win, method, args) {
        return result;
    }
    if method == "startDrag" {
        return drag_out::start_drag(win, args);
    }

    Err(format!("window.{method}: unknown method"))
}

pub(super) type DispatchResult = Option<Result<Value, String>>;

fn dispatch_geometry(win: &Window, method: &str, args: &Value) -> DispatchResult {
    match method {
        "setTitle" => Some(set_title(win, args)),
        "getTitle" => Some(Ok(Value::String(win.title()))),
        "setSize" => Some(set_size(win, args)),
        "getSize" => {
            let size = win.inner_size().to_logical::<f64>(win.scale_factor());
            Some(Ok(json!({ "width": size.width, "height": size.height })))
        },
        "getOuterSize" => {
            let size = win.outer_size().to_logical::<f64>(win.scale_factor());
            Some(Ok(json!({ "width": size.width, "height": size.height })))
        },
        "setPosition" => {
            let (x, y) = coordinates(args);
            win.set_outer_position(LogicalPosition::new(x, y));
            Some(Ok(Value::Null))
        },
        "getPosition" => {
            let position: PhysicalPosition<i32> = win.outer_position().unwrap_or_default();
            let logical = position.to_logical::<f64>(win.scale_factor());
            Some(Ok(json!({ "x": logical.x, "y": logical.y })))
        },
        "setMinSize" => {
            win.set_min_inner_size(optional_size(args));
            Some(Ok(Value::Null))
        },
        "setMaxSize" => {
            win.set_max_inner_size(optional_size(args));
            Some(Ok(Value::Null))
        },
        _ => None,
    }
}

fn set_title(win: &Window, args: &Value) -> Result<Value, String> {
    let title = args
        .get("title")
        .and_then(Value::as_str)
        .ok_or("setTitle: missing 'title'")?;
    win.set_title(title);
    Ok(Value::Null)
}

fn set_size(win: &Window, args: &Value) -> Result<Value, String> {
    let (width, height) = parse_positive_size(args)?;
    win.set_inner_size(LogicalSize::new(width, height));
    Ok(Value::Null)
}

fn optional_size(args: &Value) -> Option<LogicalSize<f64>> {
    let width = args.get("width").and_then(Value::as_f64);
    let height = args.get("height").and_then(Value::as_f64);
    width
        .zip(height)
        .map(|(width, height)| LogicalSize::new(width, height))
}

pub(super) fn coordinates(args: &Value) -> (f64, f64) {
    let x = args.get("x").and_then(Value::as_f64).unwrap_or(0.0);
    let y = args.get("y").and_then(Value::as_f64).unwrap_or(0.0);
    (x, y)
}

fn dispatch_window_configuration(win: &Window, method: &str, args: &Value) -> DispatchResult {
    match method {
        "setResizable" => {
            win.set_resizable(bool_arg(args, "resizable", true));
            Some(Ok(Value::Null))
        },
        "setClosable" => {
            win.set_closable(bool_arg(args, "closable", true));
            Some(Ok(Value::Null))
        },
        "setMaximizable" => {
            win.set_maximizable(bool_arg(args, "maximizable", true));
            Some(Ok(Value::Null))
        },
        "setMinimizable" => {
            win.set_minimizable(bool_arg(args, "minimizable", true));
            Some(Ok(Value::Null))
        },
        "setFullscreen" => {
            win.set_fullscreen(if bool_arg(args, "fullscreen", false) {
                Some(tao::window::Fullscreen::Borderless(None))
            } else {
                None
            });
            Some(Ok(Value::Null))
        },
        "setDecorations" => {
            win.set_decorations(bool_arg(args, "decorations", true));
            Some(Ok(Value::Null))
        },
        "setFocusable" => {
            win.set_focusable(bool_arg(args, "focusable", true));
            Some(Ok(Value::Null))
        },
        "setEnabled" => {
            Some(set_enabled(win, bool_arg(args, "enabled", true)).map(|()| Value::Null))
        },
        "setVisibleOnAllWorkspaces" => {
            win.set_visible_on_all_workspaces(bool_arg(args, "visible", false));
            Some(Ok(Value::Null))
        },
        "isResizable" => Some(Ok(Value::Bool(win.is_resizable()))),
        "isClosable" => Some(Ok(Value::Bool(win.is_closable()))),
        "isFocused" => Some(Ok(Value::Bool(win.is_focused()))),
        "isDecorated" => Some(Ok(Value::Bool(win.is_decorated()))),
        "scaleFactor" => Some(Ok(json!(win.scale_factor()))),
        _ => None,
    }
}

pub(super) fn bool_arg(args: &Value, key: &str, default: bool) -> bool {
    args.get(key).and_then(Value::as_bool).unwrap_or(default)
}

fn dispatch_window_action(win: &Window, method: &str) -> DispatchResult {
    match method {
        "toggleMaximize" => {
            win.set_maximized(!win.is_maximized());
            Some(Ok(Value::Null))
        },
        "minimize" => {
            win.set_minimized(true);
            Some(Ok(Value::Null))
        },
        "unminimize" => {
            win.set_minimized(false);
            Some(Ok(Value::Null))
        },
        "maximize" => {
            win.set_maximized(true);
            Some(Ok(Value::Null))
        },
        "unmaximize" => {
            win.set_maximized(false);
            Some(Ok(Value::Null))
        },
        "center" => {
            crate::window::center_window(win);
            Some(Ok(Value::Null))
        },
        "show" => {
            win.set_visible(true);
            Some(Ok(Value::Null))
        },
        "hide" => {
            win.set_visible(false);
            Some(Ok(Value::Null))
        },
        "isMaximized" => Some(Ok(Value::Bool(win.is_maximized()))),
        "isMinimized" => Some(Ok(Value::Bool(win.is_minimized()))),
        "isFullscreen" => Some(Ok(Value::Bool(win.fullscreen().is_some()))),
        "isVisible" => Some(Ok(Value::Bool(win.is_visible()))),
        "setFocus" => {
            win.set_focus();
            Some(Ok(Value::Null))
        },
        _ => None,
    }
}

fn dispatch_pointer(win: &Window, method: &str, args: &Value) -> DispatchResult {
    match method {
        "setCursorIcon" => {
            let name = args
                .get("icon")
                .and_then(Value::as_str)
                .unwrap_or("default");
            win.set_cursor_icon(parse_cursor(name));
            Some(Ok(Value::Null))
        },
        "setCursorPosition" => {
            let (x, y) = coordinates(args);
            Some(
                win.set_cursor_position(LogicalPosition::new(x, y))
                    .map(|()| Value::Null)
                    .map_err(|error| format!("setCursorPosition: {error}")),
            )
        },
        "setCursorVisible" => {
            win.set_cursor_visible(bool_arg(args, "visible", true));
            Some(Ok(Value::Null))
        },
        "setIgnoreCursorEvents" => Some(
            win.set_ignore_cursor_events(bool_arg(args, "ignore", false))
                .map(|()| Value::Null)
                .map_err(|error| format!("setIgnoreCursorEvents: {error}")),
        ),
        "startDragging" => Some(
            win.drag_window()
                .map(|()| Value::Null)
                .map_err(|error| format!("startDragging: {error}")),
        ),
        "startResizeDragging" => Some(start_resize_dragging(win, args)),
        _ => None,
    }
}

fn start_resize_dragging(win: &Window, args: &Value) -> Result<Value, String> {
    let direction = args
        .get("direction")
        .and_then(Value::as_str)
        .unwrap_or("southEast");
    win.drag_resize_window(parse_resize_direction(direction)?)
        .map_err(|error| format!("startResizeDragging: {error}"))?;
    Ok(Value::Null)
}

fn dispatch_appearance(win: &Window, method: &str, args: &Value) -> DispatchResult {
    match method {
        "setTheme" => Some(parse_theme(args).map(|theme| {
            win.set_theme(theme);
            Value::Null
        })),
        "setBackgroundColor" => {
            win.set_background_color(parse_background_color(args));
            Some(Ok(Value::Null))
        },
        "setContentProtection" => {
            win.set_content_protection(bool_arg(args, "enabled", false));
            Some(Ok(Value::Null))
        },
        "setProgressBar" => {
            win.set_progress_bar(parse_progress_bar(args));
            Some(Ok(Value::Null))
        },
        _ => None,
    }
}

fn parse_positive_size(args: &Value) -> Result<(f64, f64), String> {
    let w = args.get("width").and_then(Value::as_f64).unwrap_or(800.0);
    let h = args.get("height").and_then(Value::as_f64).unwrap_or(600.0);
    if !w.is_finite() || w <= 0.0 || !h.is_finite() || h <= 0.0 {
        return Err("setSize: width and height must be positive finite numbers".into());
    }
    Ok((w, h))
}

fn parse_theme(args: &Value) -> Result<Option<Theme>, String> {
    match args.get("theme").and_then(Value::as_str) {
        Some("light") => Ok(Some(Theme::Light)),
        Some("dark") => Ok(Some(Theme::Dark)),
        Some("system") | None => Ok(None),
        Some(other) => Err(format!("setTheme: unknown theme '{other}'")),
    }
}

fn parse_background_color(args: &Value) -> Option<(u8, u8, u8, u8)> {
    args.get("color").and_then(|v| {
        // Accept `[r,g,b,a]` or `{r,g,b,a}` — JSON-friendly both ways.
        if let Some(arr) = v.as_array() {
            if arr.len() == 4 {
                return Some((
                    arr[0].as_u64()? as u8,
                    arr[1].as_u64()? as u8,
                    arr[2].as_u64()? as u8,
                    arr[3].as_u64()? as u8,
                ));
            }
        }
        let r = v.get("r")?.as_u64()? as u8;
        let g = v.get("g")?.as_u64()? as u8;
        let b = v.get("b")?.as_u64()? as u8;
        let a = v.get("a").and_then(Value::as_u64).unwrap_or(255) as u8;
        Some((r, g, b, a))
    })
}

fn parse_progress_bar(args: &Value) -> ProgressBarState {
    let state = match args.get("state").and_then(Value::as_str).unwrap_or("none") {
        "normal" => Some(ProgressState::Normal),
        "indeterminate" => Some(ProgressState::Indeterminate),
        "paused" => Some(ProgressState::Paused),
        "error" => Some(ProgressState::Error),
        _ => Some(ProgressState::None),
    };
    let progress = args.get("progress").and_then(Value::as_u64);
    ProgressBarState {
        state,
        progress,
        desktop_filename: args
            .get("desktopFilename")
            .and_then(Value::as_str)
            .map(str::to_string),
    }
}

/// Map an API string to tao's `CursorIcon`. Unknown names fall back to
/// `Default` so apps don't hard-crash on typos.
fn parse_cursor(name: &str) -> CursorIcon {
    match name {
        "crosshair" => CursorIcon::Crosshair,
        "hand" => CursorIcon::Hand,
        "arrow" => CursorIcon::Arrow,
        "move" => CursorIcon::Move,
        "text" => CursorIcon::Text,
        "wait" => CursorIcon::Wait,
        "help" => CursorIcon::Help,
        "progress" => CursorIcon::Progress,
        "notAllowed" => CursorIcon::NotAllowed,
        "contextMenu" => CursorIcon::ContextMenu,
        "cell" => CursorIcon::Cell,
        "verticalText" => CursorIcon::VerticalText,
        "alias" => CursorIcon::Alias,
        "copy" => CursorIcon::Copy,
        "noDrop" => CursorIcon::NoDrop,
        "grab" => CursorIcon::Grab,
        "grabbing" => CursorIcon::Grabbing,
        "allScroll" => CursorIcon::AllScroll,
        "zoomIn" => CursorIcon::ZoomIn,
        "zoomOut" => CursorIcon::ZoomOut,
        "eResize" => CursorIcon::EResize,
        "nResize" => CursorIcon::NResize,
        "neResize" => CursorIcon::NeResize,
        "nwResize" => CursorIcon::NwResize,
        "sResize" => CursorIcon::SResize,
        "seResize" => CursorIcon::SeResize,
        "swResize" => CursorIcon::SwResize,
        "wResize" => CursorIcon::WResize,
        "ewResize" => CursorIcon::EwResize,
        "nsResize" => CursorIcon::NsResize,
        "neswResize" => CursorIcon::NeswResize,
        "nwseResize" => CursorIcon::NwseResize,
        "colResize" => CursorIcon::ColResize,
        "rowResize" => CursorIcon::RowResize,
        _ => CursorIcon::Default,
    }
}

fn parse_resize_direction(dir: &str) -> Result<ResizeDirection, String> {
    Ok(match dir {
        "east" => ResizeDirection::East,
        "north" => ResizeDirection::North,
        "northEast" => ResizeDirection::NorthEast,
        "northWest" => ResizeDirection::NorthWest,
        "south" => ResizeDirection::South,
        "southEast" => ResizeDirection::SouthEast,
        "southWest" => ResizeDirection::SouthWest,
        "west" => ResizeDirection::West,
        other => return Err(format!("unknown resize direction '{other}'")),
    })
}
