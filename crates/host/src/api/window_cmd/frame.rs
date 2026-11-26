//! Frame-level window controls: always-on-top levels, kiosk, simple
//! fullscreen, opacity, aspect ratio, movability, attention requests and the
//! frame state the injected drag/resize script reads.
//!
//! tao exposes part of this directly; the rest goes through the native
//! window: a Win32 subclass on Windows, `NSWindow` messages on macOS and the
//! GTK window on Linux. A call that cannot have any effect on the current OS
//! rejects with `"<method>: not supported on <os>"`.

use std::{
    collections::HashMap,
    sync::{LazyLock, Mutex},
};

use dpi::PhysicalPosition;
use serde_json::{json, Value};
use tao::window::{Fullscreen, UserAttentionType, Window, WindowId};

use super::{bool_arg, DispatchResult};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "linux")]
use linux as native;
#[cfg(target_os = "macos")]
use macos as native;
#[cfg(target_os = "windows")]
use windows as native;

pub(super) fn dispatch_frame(win: &Window, method: &str, args: &Value) -> DispatchResult {
    let result = match method {
        "setAlwaysOnTop" => set_always_on_top(win, args),
        "setAlwaysOnBottom" => {
            win.set_always_on_bottom(bool_arg(args, "bottom", false));
            Ok(Value::Null)
        },
        "isAlwaysOnTop" => Ok(Value::Bool(win.is_always_on_top())),
        "isMinimizable" => Ok(Value::Bool(win.is_minimizable())),
        "isMaximizable" => Ok(Value::Bool(win.is_maximizable())),
        "setKiosk" => {
            set_kiosk(win, bool_arg(args, "enabled", false));
            Ok(Value::Null)
        },
        "isKiosk" => Ok(Value::Bool(is_kiosk(win))),
        "setSimpleFullscreen" => {
            native::set_simple_fullscreen(win, bool_arg(args, "fullscreen", false));
            Ok(Value::Null)
        },
        "setOpacity" => parse_opacity(args)
            .and_then(|opacity| native::set_opacity(win, opacity))
            .map(|()| Value::Null),
        "setAspectRatio" => parse_aspect_ratio(args)
            .and_then(|ratio| native::set_aspect_ratio(win, ratio))
            .map(|()| Value::Null),
        "setMovable" => {
            native::set_movable(win, bool_arg(args, "movable", true)).map(|()| Value::Null)
        },
        "getInnerPosition" => Ok(inner_position(win)),
        "setCursorGrab" => win
            .set_cursor_grab(bool_arg(args, "grab", false))
            .map(|()| Value::Null)
            .map_err(|error| format!("setCursorGrab: {error}")),
        "requestAttention" => parse_attention(args).map(|kind| {
            win.request_user_attention(Some(kind));
            Value::Null
        }),
        "cancelAttention" => {
            win.request_user_attention(None);
            Ok(Value::Null)
        },
        "frameState" => Ok(frame_state(win)),
        _ => return None,
    };
    Some(result)
}

/// macOS window levels; Windows and Linux have a single "topmost" layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TopLevel {
    Floating,
    Status,
    ScreenSaver,
}

fn parse_top_level(args: &Value) -> Result<TopLevel, String> {
    match args.get("level") {
        None | Some(Value::Null) => Ok(TopLevel::Floating),
        Some(Value::String(level)) => match level.as_str() {
            "floating" => Ok(TopLevel::Floating),
            "status" => Ok(TopLevel::Status),
            "screen-saver" => Ok(TopLevel::ScreenSaver),
            other => Err(format!(
                "setAlwaysOnTop: unknown level '{other}'; use \"floating\", \"status\" or \"screen-saver\""
            )),
        },
        Some(_) => Err("setAlwaysOnTop: 'level' must be a string".to_string()),
    }
}

fn set_always_on_top(win: &Window, args: &Value) -> Result<Value, String> {
    let always = bool_arg(args, "always", false);
    let level = parse_top_level(args)?;
    win.set_always_on_top(always);
    if always {
        native::set_top_level(win, level);
    }
    Ok(Value::Null)
}

/// Windows in kiosk mode, with the always-on-top state to restore on exit.
static KIOSK: LazyLock<Mutex<HashMap<WindowId, bool>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn kiosk_windows() -> std::sync::MutexGuard<'static, HashMap<WindowId, bool>> {
    KIOSK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Kiosk mode: borderless fullscreen pinned above every other window.
pub(crate) fn set_kiosk(win: &Window, enabled: bool) {
    let mut kiosk = kiosk_windows();
    if enabled {
        kiosk
            .entry(win.id())
            .or_insert_with(|| win.is_always_on_top());
        drop(kiosk);
        win.set_fullscreen(Some(Fullscreen::Borderless(None)));
        win.set_always_on_top(true);
        native::set_top_level(win, TopLevel::Status);
    } else if let Some(was_on_top) = kiosk.remove(&win.id()) {
        drop(kiosk);
        win.set_fullscreen(None);
        win.set_always_on_top(was_on_top);
    }
}

fn is_kiosk(win: &Window) -> bool {
    win.fullscreen().is_some() && kiosk_windows().contains_key(&win.id())
}

fn parse_opacity(args: &Value) -> Result<f64, String> {
    match args.get("opacity").and_then(Value::as_f64) {
        Some(opacity) if opacity.is_finite() && (0.0..=1.0).contains(&opacity) => Ok(opacity),
        _ => Err("setOpacity: 'opacity' must be a number between 0 and 1".to_string()),
    }
}

/// `None` (or `0`) clears the constraint; otherwise a positive width / height ratio.
fn parse_aspect_ratio(args: &Value) -> Result<Option<f64>, String> {
    match args.get("ratio") {
        None | Some(Value::Null) => Ok(None),
        Some(value) => match value.as_f64() {
            Some(0.0) => Ok(None),
            Some(ratio) if ratio.is_finite() && ratio > 0.0 => Ok(Some(ratio)),
            _ => Err(
                "setAspectRatio: 'ratio' must be a positive width / height number, or null to clear"
                    .to_string(),
            ),
        },
    }
}

fn parse_attention(args: &Value) -> Result<UserAttentionType, String> {
    match args.get("kind").and_then(Value::as_str) {
        None | Some("informational") => Ok(UserAttentionType::Informational),
        Some("critical") => Ok(UserAttentionType::Critical),
        Some(other) => Err(format!(
            "requestAttention: unknown kind '{other}'; use \"informational\" or \"critical\""
        )),
    }
}

fn inner_position(win: &Window) -> Value {
    let position: PhysicalPosition<i32> = win.inner_position().unwrap_or_default();
    let logical = position.to_logical::<f64>(win.scale_factor());
    json!({ "x": logical.x, "y": logical.y })
}

/// Read by the injected frame script: whether a mouse-down on the page
/// border should start a native resize (frameless, resizable, restored).
fn frame_state(win: &Window) -> Value {
    let edge_resizable = native::SUPPORTS_EDGE_RESIZE
        && !win.is_decorated()
        && win.is_resizable()
        && !win.is_maximized()
        && win.fullscreen().is_none();
    json!({ "edgeResizable": edge_resizable })
}

/// Fit a proposed outer window rectangle to a client-area aspect ratio while
/// the user drags `edge`. `frame` is the non-client width / height. The edge
/// being dragged keeps its position; the opposite one moves.
#[cfg_attr(not(any(target_os = "windows", test)), allow(dead_code))]
pub(super) fn fit_aspect(
    edge: SizingEdge,
    rect: (i32, i32, i32, i32),
    frame: (i32, i32),
    ratio: f64,
) -> (i32, i32, i32, i32) {
    let (left, top, right, bottom) = rect;
    let client_w = f64::from((right - left - frame.0).max(1));
    let client_h = f64::from((bottom - top - frame.1).max(1));
    #[allow(clippy::cast_possible_truncation)]
    let to_px = |value: f64| value.round().clamp(1.0, f64::from(i32::MAX / 2)) as i32;
    match edge {
        SizingEdge::Top | SizingEdge::Bottom => {
            let width = to_px(client_h * ratio) + frame.0;
            (left, top, left + width, bottom)
        },
        SizingEdge::Left | SizingEdge::Right | SizingEdge::BottomLeft | SizingEdge::BottomRight => {
            let height = to_px(client_w / ratio) + frame.1;
            (left, top, right, top + height)
        },
        SizingEdge::TopLeft | SizingEdge::TopRight => {
            let height = to_px(client_w / ratio) + frame.1;
            (left, bottom - height, right, bottom)
        },
    }
}

/// The window edge or corner being dragged during a resize.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub(super) enum SizingEdge {
    Left,
    Right,
    Top,
    TopLeft,
    TopRight,
    Bottom,
    BottomLeft,
    BottomRight,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn top_level_defaults_to_floating_and_rejects_unknown_levels() {
        assert_eq!(parse_top_level(&json!({})), Ok(TopLevel::Floating));
        assert_eq!(
            parse_top_level(&json!({ "level": "screen-saver" })),
            Ok(TopLevel::ScreenSaver)
        );
        assert_eq!(
            parse_top_level(&json!({ "level": "status" })),
            Ok(TopLevel::Status)
        );
        assert!(parse_top_level(&json!({ "level": "modal" }))
            .unwrap_err()
            .contains("unknown level 'modal'"));
        assert!(parse_top_level(&json!({ "level": 3 })).is_err());
    }

    #[test]
    fn opacity_must_be_between_zero_and_one() {
        assert_eq!(parse_opacity(&json!({ "opacity": 0.5 })), Ok(0.5));
        assert_eq!(parse_opacity(&json!({ "opacity": 0 })), Ok(0.0));
        assert_eq!(parse_opacity(&json!({ "opacity": 1 })), Ok(1.0));
        assert!(parse_opacity(&json!({ "opacity": 1.5 })).is_err());
        assert!(parse_opacity(&json!({ "opacity": -0.1 })).is_err());
        assert!(parse_opacity(&json!({ "opacity": "1" })).is_err());
        assert!(parse_opacity(&json!({})).is_err());
    }

    #[test]
    fn aspect_ratio_accepts_positive_numbers_and_clears_on_null_or_zero() {
        assert_eq!(parse_aspect_ratio(&json!({ "ratio": 1.5 })), Ok(Some(1.5)));
        assert_eq!(parse_aspect_ratio(&json!({ "ratio": null })), Ok(None));
        assert_eq!(parse_aspect_ratio(&json!({})), Ok(None));
        assert_eq!(parse_aspect_ratio(&json!({ "ratio": 0 })), Ok(None));
        assert!(parse_aspect_ratio(&json!({ "ratio": -1 })).is_err());
        assert!(parse_aspect_ratio(&json!({ "ratio": "16/9" })).is_err());
    }

    #[test]
    fn attention_kind_defaults_to_informational() {
        assert_eq!(
            parse_attention(&json!({})),
            Ok(UserAttentionType::Informational)
        );
        assert_eq!(
            parse_attention(&json!({ "kind": "critical" })),
            Ok(UserAttentionType::Critical)
        );
        assert!(parse_attention(&json!({ "kind": "loud" })).is_err());
    }

    #[test]
    fn fit_aspect_keeps_the_dragged_edge() {
        // 16 px horizontal / 39 px vertical frame, 2:1 ratio.
        let frame = (16, 39);
        // Dragging the right edge to a 816-wide window: client 800 → height 400.
        assert_eq!(
            fit_aspect(SizingEdge::Right, (0, 0, 816, 100), frame, 2.0),
            (0, 0, 816, 439)
        );
        // Dragging the bottom edge: client height 300 → width 600.
        assert_eq!(
            fit_aspect(SizingEdge::Bottom, (10, 10, 50, 349), frame, 2.0),
            (10, 10, 626, 349)
        );
        // Dragging the top-left corner moves the top edge, bottom stays put.
        assert_eq!(
            fit_aspect(SizingEdge::TopLeft, (100, 0, 916, 1000), frame, 2.0),
            (100, 561, 916, 1000)
        );
    }
}
