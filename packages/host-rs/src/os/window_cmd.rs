use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use tao::{
    dpi::{LogicalPosition, LogicalSize, PhysicalPosition},
    window::{CursorIcon, ResizeDirection, Window},
};

use crate::window;

/// Set by `tyndWindow.cancelClose()` during a `window:close-requested` handler.
/// Polled 500ms later by the event loop before actually hiding / exiting.
static CLOSE_CANCELLED: AtomicBool = AtomicBool::new(false);

pub fn reset_close_cancel() {
    CLOSE_CANCELLED.store(false, Ordering::SeqCst);
}

pub fn close_cancelled() -> bool {
    CLOSE_CANCELLED.load(Ordering::SeqCst)
}

/// Dispatch a window command synchronously on the main (event-loop) thread.
pub fn dispatch(win: &Window, method: &str, args: &Value) -> Result<Value, String> {
    match method {
        "setTitle" => {
            let title = args
                .get("title")
                .and_then(|t| t.as_str())
                .ok_or("setTitle: missing 'title'")?;
            win.set_title(title);
            Ok(Value::Null)
        },
        "getTitle" => Ok(Value::String(win.title())),

        "setSize" => {
            let w = args.get("width").and_then(Value::as_f64).unwrap_or(800.0);
            let h = args.get("height").and_then(Value::as_f64).unwrap_or(600.0);
            if !w.is_finite() || w <= 0.0 || !h.is_finite() || h <= 0.0 {
                return Err("setSize: width and height must be positive finite numbers".into());
            }
            win.set_inner_size(LogicalSize::new(w, h));
            Ok(Value::Null)
        },
        "getSize" => {
            let s = win.inner_size().to_logical::<f64>(win.scale_factor());
            Ok(json!({ "width": s.width, "height": s.height }))
        },
        "getOuterSize" => {
            let s = win.outer_size().to_logical::<f64>(win.scale_factor());
            Ok(json!({ "width": s.width, "height": s.height }))
        },
        "setPosition" => {
            let x = args.get("x").and_then(Value::as_f64).unwrap_or(0.0);
            let y = args.get("y").and_then(Value::as_f64).unwrap_or(0.0);
            win.set_outer_position(LogicalPosition::new(x, y));
            Ok(Value::Null)
        },
        "getPosition" => {
            let p: PhysicalPosition<i32> = win.outer_position().unwrap_or_default();
            let l = p.to_logical::<f64>(win.scale_factor());
            Ok(json!({ "x": l.x, "y": l.y }))
        },
        "setMinSize" => {
            let w = args.get("width").and_then(Value::as_f64);
            let h = args.get("height").and_then(Value::as_f64);
            win.set_min_inner_size(w.zip(h).map(|(w, h)| LogicalSize::new(w, h)));
            Ok(Value::Null)
        },
        "setMaxSize" => {
            let w = args.get("width").and_then(Value::as_f64);
            let h = args.get("height").and_then(Value::as_f64);
            win.set_max_inner_size(w.zip(h).map(|(w, h)| LogicalSize::new(w, h)));
            Ok(Value::Null)
        },
        "setResizable" => {
            let b = args
                .get("resizable")
                .and_then(Value::as_bool)
                .unwrap_or(true);
            win.set_resizable(b);
            Ok(Value::Null)
        },
        "setClosable" => {
            let b = args
                .get("closable")
                .and_then(Value::as_bool)
                .unwrap_or(true);
            win.set_closable(b);
            Ok(Value::Null)
        },
        "setMaximizable" => {
            let b = args
                .get("maximizable")
                .and_then(Value::as_bool)
                .unwrap_or(true);
            win.set_maximizable(b);
            Ok(Value::Null)
        },
        "setMinimizable" => {
            let b = args
                .get("minimizable")
                .and_then(Value::as_bool)
                .unwrap_or(true);
            win.set_minimizable(b);
            Ok(Value::Null)
        },
        "toggleMaximize" => {
            win.set_maximized(!win.is_maximized());
            Ok(Value::Null)
        },
        "isResizable" => Ok(Value::Bool(win.is_resizable())),
        "isClosable" => Ok(Value::Bool(win.is_closable())),
        "isFocused" => Ok(Value::Bool(win.is_focused())),
        "isDecorated" => Ok(Value::Bool(win.is_decorated())),
        "scaleFactor" => Ok(json!(win.scale_factor())),

        "minimize" => {
            win.set_minimized(true);
            Ok(Value::Null)
        },
        "unminimize" => {
            win.set_minimized(false);
            Ok(Value::Null)
        },
        "maximize" => {
            win.set_maximized(true);
            Ok(Value::Null)
        },
        "unmaximize" => {
            win.set_maximized(false);
            Ok(Value::Null)
        },
        "center" => {
            window::center_window(win);
            Ok(Value::Null)
        },

        "setFullscreen" => {
            let full = args
                .get("fullscreen")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            win.set_fullscreen(if full {
                Some(tao::window::Fullscreen::Borderless(None))
            } else {
                None
            });
            Ok(Value::Null)
        },

        "setAlwaysOnTop" => {
            let always = args.get("always").and_then(Value::as_bool).unwrap_or(false);
            win.set_always_on_top(always);
            Ok(Value::Null)
        },

        "setDecorations" => {
            let dec = args
                .get("decorations")
                .and_then(Value::as_bool)
                .unwrap_or(true);
            win.set_decorations(dec);
            Ok(Value::Null)
        },

        "show" => {
            win.set_visible(true);
            Ok(Value::Null)
        },
        "hide" => {
            win.set_visible(false);
            Ok(Value::Null)
        },

        "isMaximized" => Ok(Value::Bool(win.is_maximized())),
        "isMinimized" => Ok(Value::Bool(win.is_minimized())),
        "isFullscreen" => Ok(Value::Bool(win.fullscreen().is_some())),
        "isVisible" => Ok(Value::Bool(win.is_visible())),

        "cancelClose" => {
            CLOSE_CANCELLED.store(true, Ordering::SeqCst);
            Ok(Value::Null)
        },

        "monitors" => Ok(list_monitors(win)),
        "primaryMonitor" => Ok(primary_monitor(win)),
        "currentMonitor" => Ok(current_monitor(win)),
        "monitorFromPoint" => {
            let x = args.get("x").and_then(Value::as_f64).unwrap_or(0.0);
            let y = args.get("y").and_then(Value::as_f64).unwrap_or(0.0);
            Ok(monitor_from_point(win, x, y))
        },
        "globalCursorPosition" => {
            let pos = win
                .cursor_position()
                .map_err(|e| format!("globalCursorPosition: {e}"))?;
            Ok(json!({ "x": pos.x, "y": pos.y }))
        },

        "setFocus" => {
            win.set_focus();
            Ok(Value::Null)
        },
        "requestAttention" => {
            // Cross-OS "flash the taskbar / bounce the dock" — platform
            // independently handled by tao's UserAttentionType::Informational.
            win.request_user_attention(Some(tao::window::UserAttentionType::Informational));
            Ok(Value::Null)
        },

        "setCursorIcon" => {
            let name = args
                .get("icon")
                .and_then(Value::as_str)
                .unwrap_or("default");
            win.set_cursor_icon(parse_cursor(name));
            Ok(Value::Null)
        },
        "setCursorPosition" => {
            let x = args.get("x").and_then(Value::as_f64).unwrap_or(0.0);
            let y = args.get("y").and_then(Value::as_f64).unwrap_or(0.0);
            win.set_cursor_position(LogicalPosition::new(x, y))
                .map_err(|e| format!("setCursorPosition: {e}"))?;
            Ok(Value::Null)
        },
        "setCursorVisible" => {
            let visible = args.get("visible").and_then(Value::as_bool).unwrap_or(true);
            win.set_cursor_visible(visible);
            Ok(Value::Null)
        },
        "setIgnoreCursorEvents" => {
            let ignore = args.get("ignore").and_then(Value::as_bool).unwrap_or(false);
            win.set_ignore_cursor_events(ignore)
                .map_err(|e| format!("setIgnoreCursorEvents: {e}"))?;
            Ok(Value::Null)
        },
        "startDragging" => {
            win.drag_window()
                .map_err(|e| format!("startDragging: {e}"))?;
            Ok(Value::Null)
        },
        "startResizeDragging" => {
            let dir = args
                .get("direction")
                .and_then(Value::as_str)
                .unwrap_or("southEast");
            win.drag_resize_window(parse_resize_direction(dir)?)
                .map_err(|e| format!("startResizeDragging: {e}"))?;
            Ok(Value::Null)
        },

        "setTheme" => {
            let theme = args.get("theme").and_then(Value::as_str);
            let t = match theme {
                Some("light") => Some(tao::window::Theme::Light),
                Some("dark") => Some(tao::window::Theme::Dark),
                Some("system") | None => None,
                Some(other) => return Err(format!("setTheme: unknown theme '{other}'")),
            };
            win.set_theme(t);
            Ok(Value::Null)
        },
        "setBackgroundColor" => {
            let rgba = args.get("color").and_then(|v| {
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
            });
            win.set_background_color(rgba);
            Ok(Value::Null)
        },
        "setContentProtection" => {
            let enabled = args
                .get("enabled")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            win.set_content_protection(enabled);
            Ok(Value::Null)
        },
        "setProgressBar" => {
            let state = match args.get("state").and_then(Value::as_str).unwrap_or("none") {
                "normal" => Some(tao::window::ProgressState::Normal),
                "indeterminate" => Some(tao::window::ProgressState::Indeterminate),
                "paused" => Some(tao::window::ProgressState::Paused),
                "error" => Some(tao::window::ProgressState::Error),
                _ => Some(tao::window::ProgressState::None),
            };
            let progress = args.get("progress").and_then(Value::as_u64);
            win.set_progress_bar(tao::window::ProgressBarState {
                state,
                progress,
                desktop_filename: args
                    .get("desktopFilename")
                    .and_then(Value::as_str)
                    .map(str::to_string),
            });
            Ok(Value::Null)
        },
        "setSkipTaskbar" => {
            let skip = args.get("skip").and_then(Value::as_bool).unwrap_or(false);
            set_skip_taskbar(win, skip)
        },
        "setBadge" => {
            let label = args.get("label").and_then(Value::as_str);
            let count = args.get("count").and_then(Value::as_i64);
            set_badge(win, label, count);
            Ok(Value::Null)
        },
        "setFocusable" => {
            let b = args
                .get("focusable")
                .and_then(Value::as_bool)
                .unwrap_or(true);
            win.set_focusable(b);
            Ok(Value::Null)
        },
        "setEnabled" => {
            let enabled = args.get("enabled").and_then(Value::as_bool).unwrap_or(true);
            set_enabled(win, enabled);
            Ok(Value::Null)
        },
        "setVisibleOnAllWorkspaces" => {
            let v = args
                .get("visible")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            win.set_visible_on_all_workspaces(v);
            Ok(Value::Null)
        },
        "setShadow" => {
            let enabled = args.get("enabled").and_then(Value::as_bool).unwrap_or(true);
            set_has_shadow(win, enabled);
            Ok(Value::Null)
        },
        "setTitlebarTransparent" => {
            let t = args
                .get("transparent")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            set_titlebar_transparent(win, t);
            Ok(Value::Null)
        },
        "setFullsizeContentView" => {
            let f = args
                .get("fullsize")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            set_fullsize_content_view(win, f);
            Ok(Value::Null)
        },
        "setTrafficLightInset" => {
            let x = args.get("x").and_then(Value::as_f64).unwrap_or(0.0);
            let y = args.get("y").and_then(Value::as_f64).unwrap_or(0.0);
            set_traffic_light_inset(win, x, y);
            Ok(Value::Null)
        },
        "setSystemBackdrop" => {
            let kind = args.get("kind").and_then(Value::as_str).unwrap_or("auto");
            set_system_backdrop(win, kind)?;
            Ok(Value::Null)
        },
        "setWindowIcon" => {
            // Passing null clears the icon; a path reloads it.
            match args.get("path").and_then(Value::as_str) {
                Some(path) => {
                    let icon = crate::os::icon::load_tao(path)?;
                    win.set_window_icon(Some(icon));
                },
                None => win.set_window_icon(None),
            }
            Ok(Value::Null)
        },

        _ => Err(format!("window.{method}: unknown method")),
    }
}

#[cfg(target_os = "windows")]
fn set_skip_taskbar(win: &Window, skip: bool) -> Result<Value, String> {
    use tao::platform::windows::WindowExtWindows;
    win.set_skip_taskbar(skip)
        .map_err(|e| format!("setSkipTaskbar: {e}"))?;
    Ok(Value::Null)
}

#[cfg(all(unix, not(target_os = "macos")))]
fn set_skip_taskbar(win: &Window, skip: bool) -> Result<Value, String> {
    use tao::platform::unix::WindowExtUnix;
    win.set_skip_taskbar(skip)
        .map_err(|e| format!("setSkipTaskbar: {e}"))?;
    Ok(Value::Null)
}

#[cfg(target_os = "macos")]
#[allow(clippy::needless_pass_by_value, clippy::unnecessary_wraps)]
fn set_skip_taskbar(_win: &Window, _skip: bool) -> Result<Value, String> {
    // macOS has no per-window Dock hide — see `app.setDockVisible`.
    // Signature matches the Windows + Linux branches so the caller can
    // `?`-propagate uniformly.
    Ok(Value::Null)
}

#[cfg(target_os = "macos")]
fn set_badge(win: &Window, label: Option<&str>, _count: Option<i64>) {
    use tao::platform::macos::WindowExtMacOS;
    win.set_badge_label(label.map(str::to_string));
}

#[cfg(all(unix, not(target_os = "macos")))]
fn set_badge(win: &Window, _label: Option<&str>, count: Option<i64>) {
    use tao::platform::unix::WindowExtUnix;
    win.set_badge_count(count, None);
}

#[cfg(target_os = "windows")]
fn set_badge(_win: &Window, _label: Option<&str>, _count: Option<i64>) {
    // Windows uses taskbar overlay icons instead — not wired yet.
}

#[cfg(target_os = "windows")]
fn set_enabled(win: &Window, enabled: bool) {
    use tao::platform::windows::WindowExtWindows;
    win.set_enable(enabled);
}

#[cfg(not(target_os = "windows"))]
fn set_enabled(_win: &Window, _enabled: bool) {
    // No cross-OS primitive — macOS / Linux apps emulate "disabled" with
    // CSS pointer-events: none + an overlay, since users expect the window
    // to stay dragggable and closable regardless.
}

#[cfg(target_os = "macos")]
fn set_has_shadow(win: &Window, enabled: bool) {
    use tao::platform::macos::WindowExtMacOS;
    win.set_has_shadow(enabled);
}

#[cfg(not(target_os = "macos"))]
fn set_has_shadow(_win: &Window, _enabled: bool) {
    // Windows / Linux: no runtime API to toggle the native drop shadow.
    // Callers control this via the `decorations` flag at build time.
}

#[cfg(target_os = "macos")]
fn set_titlebar_transparent(win: &Window, transparent: bool) {
    use tao::platform::macos::WindowExtMacOS;
    win.set_titlebar_transparent(transparent);
}
#[cfg(not(target_os = "macos"))]
fn set_titlebar_transparent(_win: &Window, _transparent: bool) {}

#[cfg(target_os = "macos")]
fn set_fullsize_content_view(win: &Window, fullsize: bool) {
    use tao::platform::macos::WindowExtMacOS;
    win.set_fullsize_content_view(fullsize);
}
#[cfg(not(target_os = "macos"))]
fn set_fullsize_content_view(_win: &Window, _fullsize: bool) {}

#[cfg(target_os = "macos")]
fn set_traffic_light_inset(win: &Window, x: f64, y: f64) {
    use tao::platform::macos::WindowExtMacOS;
    win.set_traffic_light_inset(LogicalPosition::new(x, y));
}
#[cfg(not(target_os = "macos"))]
fn set_traffic_light_inset(_win: &Window, _x: f64, _y: f64) {}

#[cfg(target_os = "windows")]
fn set_system_backdrop(win: &Window, kind: &str) -> Result<(), String> {
    // DWMWA_SYSTEMBACKDROP_TYPE = 38 (Windows 11 22000+).
    // Values: 0 Auto, 1 None, 2 Mainwindow (Mica), 3 TransientWindow
    // (Acrylic / background blur), 4 TabbedWindow (Tabbed).
    use tao::platform::windows::WindowExtWindows as _;
    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::Graphics::Dwm::DwmSetWindowAttribute;

    const DWMWA_SYSTEMBACKDROP_TYPE: u32 = 38;
    let value: i32 = match kind {
        "none" => 1,
        "mica" => 2,
        "acrylic" => 3,
        "tabbed" => 4,
        _ => 0,
    };
    let hwnd = win.hwnd() as HWND;
    let value_ptr = std::ptr::addr_of!(value).cast();
    let hr = unsafe {
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_SYSTEMBACKDROP_TYPE,
            value_ptr,
            size_of::<i32>() as u32,
        )
    };
    if hr < 0 {
        return Err(format!(
            "setSystemBackdrop: DwmSetWindowAttribute hr=0x{hr:x}"
        ));
    }
    Ok(())
}

#[cfg(not(target_os = "windows"))]
#[allow(clippy::unnecessary_wraps)]
fn set_system_backdrop(_win: &Window, _kind: &str) -> Result<(), String> {
    Ok(())
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

fn monitor_to_json(m: &tao::monitor::MonitorHandle, is_primary: bool) -> Value {
    let size = m.size();
    let pos = m.position();
    json!({
        "name": m.name(),
        "position": { "x": pos.x, "y": pos.y },
        "size": { "width": size.width, "height": size.height },
        "scale": m.scale_factor(),
        "isPrimary": is_primary,
    })
}

fn list_monitors(win: &Window) -> Value {
    let primary_name = win.primary_monitor().and_then(|p| p.name());
    let monitors: Vec<Value> = win
        .available_monitors()
        .map(|m| {
            let is_primary = primary_name
                .as_ref()
                .is_some_and(|p| m.name().as_ref() == Some(p));
            monitor_to_json(&m, is_primary)
        })
        .collect();
    Value::Array(monitors)
}

fn primary_monitor(win: &Window) -> Value {
    win.primary_monitor()
        .map_or(Value::Null, |m| monitor_to_json(&m, true))
}

fn current_monitor(win: &Window) -> Value {
    let primary_name = win.primary_monitor().and_then(|p| p.name());
    win.current_monitor().map_or(Value::Null, |m| {
        let is_primary = primary_name
            .as_ref()
            .is_some_and(|p| m.name().as_ref() == Some(p));
        monitor_to_json(&m, is_primary)
    })
}

fn monitor_from_point(win: &Window, x: f64, y: f64) -> Value {
    let primary_name = win.primary_monitor().and_then(|p| p.name());
    win.monitor_from_point(x, y).map_or(Value::Null, |m| {
        let is_primary = primary_name
            .as_ref()
            .is_some_and(|p| m.name().as_ref() == Some(p));
        monitor_to_json(&m, is_primary)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancel_flag_roundtrip() {
        reset_close_cancel();
        assert!(!close_cancelled());
        CLOSE_CANCELLED.store(true, Ordering::SeqCst);
        assert!(close_cancelled());
        reset_close_cancel();
        assert!(!close_cancelled());
    }
}
