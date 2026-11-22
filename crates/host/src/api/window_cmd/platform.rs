#[cfg(target_os = "windows")]
use std::mem::size_of;

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
        "setSystemBackdrop" => {
            let kind = args.get("kind").and_then(Value::as_str).unwrap_or("auto");
            Some(set_system_backdrop(win, kind).map(|()| Value::Null))
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

// Unsupported-platform rule (same as `setSystemBackdrop`): a call that
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

#[cfg(target_os = "windows")]
fn set_badge(_win: &Window, _label: Option<&str>, _count: Option<i64>) -> Result<(), String> {
    // Windows uses taskbar overlay icons instead — not wired yet.
    Err("setBadge: not supported on Windows".to_string())
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
    use windows_sys::Win32::{Foundation::HWND, Graphics::Dwm::DwmSetWindowAttribute};

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
    // SAFETY: `DwmSetWindowAttribute` takes a `*const c_void` pointing
    // at `DWORD`-sized data; `addr_of!(value)` yields a properly-aligned
    // pointer to an `i32` local that outlives the call (function returns
    // after `hr` is computed). `hwnd` is a valid HWND owned by `win`.
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
fn set_system_backdrop(_win: &Window, _kind: &str) -> Result<(), String> {
    // Silent `Ok(())` would let JS callers think the visual change took
    // effect when nothing happened. Surface it as an error so the caller
    // can feature-detect or fall back to its own styling.
    Err("setSystemBackdrop: only supported on Windows".to_string())
}
