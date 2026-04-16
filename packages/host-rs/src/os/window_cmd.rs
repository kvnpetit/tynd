use serde_json::Value;
use tao::{dpi::LogicalSize, window::Window};

use crate::window;

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
            let w = args.get("width").and_then(|v| v.as_f64()).unwrap_or(800.0);
            let h = args.get("height").and_then(|v| v.as_f64()).unwrap_or(600.0);
            if !w.is_finite() || w <= 0.0 || !h.is_finite() || h <= 0.0 {
                return Err("setSize: width and height must be positive finite numbers".into());
            }
            win.set_inner_size(LogicalSize::new(w, h));
            Ok(Value::Null)
        },

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
                .and_then(|f| f.as_bool())
                .unwrap_or(false);
            win.set_fullscreen(if full {
                Some(tao::window::Fullscreen::Borderless(None))
            } else {
                None
            });
            Ok(Value::Null)
        },

        "setAlwaysOnTop" => {
            let always = args
                .get("always")
                .and_then(|a| a.as_bool())
                .unwrap_or(false);
            win.set_always_on_top(always);
            Ok(Value::Null)
        },

        "setDecorations" => {
            let dec = args
                .get("decorations")
                .and_then(|d| d.as_bool())
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

        _ => Err(format!("window.{method}: unknown method")),
    }
}
