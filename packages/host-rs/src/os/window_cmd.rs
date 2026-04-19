use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use tao::{
    dpi::{LogicalPosition, LogicalSize, PhysicalPosition},
    window::Window,
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

        _ => Err(format!("window.{method}: unknown method")),
    }
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
