use serde_json::Value;
use std::sync::atomic::{AtomicBool, Ordering};
use tao::{dpi::LogicalSize, window::Window};

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

        _ => Err(format!("window.{method}: unknown method")),
    }
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
