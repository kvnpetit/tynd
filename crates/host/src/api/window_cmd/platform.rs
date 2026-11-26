#[cfg(target_os = "macos")]
use dpi::LogicalPosition;
use serde_json::Value;
use tao::window::Window;

use super::{bool_arg, coordinates, DispatchResult};
pub(super) fn dispatch_platform(win: &Window, method: &str, args: &Value) -> DispatchResult {
    match method {
        "setSkipTaskbar" => Some(set_skip_taskbar(win, bool_arg(args, "skip", false))),
        "setBadge" => {
            let label = args.get("label").and_then(Value::as_str);
            let count = args.get("count").and_then(Value::as_i64);
            Some(set_badge(win, label, count).map(|()| Value::Null))
        },
        "setShadow" => {
            set_has_shadow(win, bool_arg(args, "enabled", true));
            Some(Ok(Value::Null))
        },
        "setTitlebarTransparent" => {
            set_titlebar_transparent(win, bool_arg(args, "transparent", false));
            Some(Ok(Value::Null))
        },
        "setFullsizeContentView" => {
            set_fullsize_content_view(win, bool_arg(args, "fullsize", false));
            Some(Ok(Value::Null))
        },
        "setTrafficLightInset" => {
            let (x, y) = coordinates(args);
            set_traffic_light_inset(win, x, y);
            Some(Ok(Value::Null))
        },
        "setEffects" => {
            Some(crate::platform::window_effects::set_effects(win, args).map(|()| Value::Null))
        },
        "setWindowIcon" => Some(set_window_icon(win, args)),
        _ => None,
    }
}

fn set_window_icon(win: &Window, args: &Value) -> Result<Value, String> {
    // Passing null clears the icon; a path reloads it.
    match args.get("path").and_then(Value::as_str) {
        Some(path) => {
            let icon = crate::platform::icon::load_tao(path)?;
            win.set_window_icon(Some(icon));
        },
        None => win.set_window_icon(None),
    }
    Ok(Value::Null)
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
fn set_skip_taskbar(_win: &Window, _skip: bool) -> Result<Value, String> {
    // macOS has no per-window Dock hide — see `app.setDockVisible`.
    // Signature matches the Windows + Linux branches so the caller can
    // `?`-propagate uniformly.
    Ok(Value::Null)
}

// Unsupported-platform rule (same as `setEffects`): a call that
// cannot have any visible effect rejects, so callers can feature-detect
// instead of believing a no-op worked.

/// macOS shows a text label; a bare `count` is shown as its decimal label.
#[cfg(target_os = "macos")]
fn set_badge(win: &Window, label: Option<&str>, count: Option<i64>) -> Result<(), String> {
    use tao::platform::macos::WindowExtMacOS;
    let label = label
        .map(str::to_string)
        .or_else(|| count.map(|n| n.to_string()));
    win.set_badge_label(label);
    Ok(())
}

/// Linux (Unity launcher API) shows a number only.
#[cfg(all(unix, not(target_os = "macos")))]
fn set_badge(win: &Window, label: Option<&str>, count: Option<i64>) -> Result<(), String> {
    use tao::platform::unix::WindowExtUnix;
    if count.is_none() && label.is_some() {
        return Err("setBadge: text labels are not supported on Linux; pass `count`".to_string());
    }
    win.set_badge_count(count, None);
    Ok(())
}

/// Windows draws `label` (up to 3 characters) or `count` into a taskbar
/// overlay icon.
#[cfg(target_os = "windows")]
fn set_badge(win: &Window, label: Option<&str>, count: Option<i64>) -> Result<(), String> {
    use crate::platform::taskbar_badge;
    use tao::platform::windows::WindowExtWindows as _;
    let text = taskbar_badge::badge_text(label, count)?;
    taskbar_badge::set_overlay(win.hwnd() as _, text.as_deref())
}

#[cfg(target_os = "windows")]
pub(super) fn set_enabled(win: &Window, enabled: bool) -> Result<(), String> {
    use tao::platform::windows::WindowExtWindows;
    win.set_enable(enabled);
    Ok(())
}

#[cfg(not(target_os = "windows"))]
pub(super) fn set_enabled(_win: &Window, _enabled: bool) -> Result<(), String> {
    // No cross-OS primitive — macOS / Linux apps emulate "disabled" with
    // CSS pointer-events: none + an overlay, since users expect the window
    // to stay draggable and closable regardless.
    Err("setEnabled: only supported on Windows".to_string())
}

#[cfg(target_os = "macos")]
fn set_has_shadow(win: &Window, enabled: bool) {
    use tao::platform::macos::WindowExtMacOS;
    win.set_has_shadow(enabled);
}

/// Windows: the DWM shadow of an undecorated window (decorated windows
/// always keep theirs).
#[cfg(target_os = "windows")]
fn set_has_shadow(win: &Window, enabled: bool) {
    use tao::platform::windows::WindowExtWindows;
    win.set_undecorated_shadow(enabled);
}

#[cfg(all(unix, not(target_os = "macos")))]
fn set_has_shadow(_win: &Window, _enabled: bool) {
    // Linux: the window manager / compositor owns the shadow.
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
