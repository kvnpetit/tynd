//! Win32 side of the frame controls. Opacity, aspect ratio and movability
//! live in a per-window subclass so they survive tao rewriting the window
//! styles (every `setDecorations` / `setAlwaysOnTop` resets `GWL_EXSTYLE`).

use std::{
    collections::HashMap,
    sync::{LazyLock, Mutex, MutexGuard, PoisonError},
};

use tao::{
    platform::windows::WindowExtWindows,
    window::{Fullscreen, Window},
};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM},
    UI::{
        Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
        WindowsAndMessaging::{
            GetClientRect, GetWindowLongPtrW, GetWindowRect, SetLayeredWindowAttributes,
            SetWindowLongPtrW, GWL_EXSTYLE, LWA_ALPHA, SC_MOVE, STYLESTRUCT, WMSZ_BOTTOM,
            WMSZ_BOTTOMLEFT, WMSZ_BOTTOMRIGHT, WMSZ_LEFT, WMSZ_RIGHT, WMSZ_TOP, WMSZ_TOPLEFT,
            WMSZ_TOPRIGHT, WM_NCDESTROY, WM_SIZING, WM_STYLECHANGING, WM_SYSCOMMAND, WS_EX_LAYERED,
        },
    },
};

use super::{fit_aspect, SizingEdge, TopLevel};

/// `true`: tao's `drag_resize_window` works on frameless windows.
pub(super) const SUPPORTS_EDGE_RESIZE: bool = true;

/// Subclass id ("tynd").
const SUBCLASS_ID: usize = 0x7479_6E64;

#[derive(Debug, Default, Clone, Copy)]
struct FrameState {
    /// Alpha below 255 keeps `WS_EX_LAYERED` set.
    alpha: Option<u8>,
    aspect: Option<f64>,
    immovable: bool,
}

/// Keyed by HWND; an entry exists exactly while the subclass is installed.
static FRAMES: LazyLock<Mutex<HashMap<isize, FrameState>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn frames() -> MutexGuard<'static, HashMap<isize, FrameState>> {
    FRAMES.lock().unwrap_or_else(PoisonError::into_inner)
}

fn frame_of(hwnd: HWND) -> Option<FrameState> {
    frames().get(&(hwnd as isize)).copied()
}

/// Update the window's frame state, installing the subclass on first use.
fn update(win: &Window, apply: impl FnOnce(&mut FrameState)) -> Result<(), String> {
    let key = win.hwnd();
    let installed = {
        let mut map = frames();
        let installed = map.contains_key(&key);
        apply(map.entry(key).or_default());
        installed
    };
    if !installed {
        // SAFETY: `key` is the live HWND owned by `win`, called on the
        // window's (main) thread; `frame_proc` matches `SUBCLASSPROC`.
        let ok = unsafe { SetWindowSubclass(key as HWND, Some(frame_proc), SUBCLASS_ID, 0) };
        if ok == 0 {
            frames().remove(&key);
            return Err("window subclass could not be installed".to_string());
        }
    }
    Ok(())
}

unsafe extern "system" fn frame_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    _data: usize,
) -> LRESULT {
    match msg {
        WM_SIZING => {
            if let (Some(ratio), Some(edge)) = (
                frame_of(hwnd).and_then(|state| state.aspect),
                sizing_edge(wparam),
            ) {
                // SAFETY: for WM_SIZING, `lparam` points at the proposed
                // window RECT, writable for the duration of the message.
                let rect = unsafe { &mut *(lparam as *mut RECT) };
                let frame = non_client_size(hwnd);
                let (left, top, right, bottom) = fit_aspect(
                    edge,
                    (rect.left, rect.top, rect.right, rect.bottom),
                    frame,
                    ratio,
                );
                *rect = RECT {
                    left,
                    top,
                    right,
                    bottom,
                };
            }
        },
        WM_SYSCOMMAND => {
            if (wparam & 0xFFF0) == SC_MOVE as usize
                && frame_of(hwnd).is_some_and(|state| state.immovable)
            {
                return 0;
            }
        },
        WM_STYLECHANGING => {
            #[allow(clippy::cast_possible_wrap)]
            let is_ex_style = wparam as i32 == GWL_EXSTYLE;
            if is_ex_style && frame_of(hwnd).is_some_and(|state| state.alpha.is_some()) {
                // SAFETY: for WM_STYLECHANGING, `lparam` points at a
                // writable STYLESTRUCT describing the pending change.
                let style = unsafe { &mut *(lparam as *mut STYLESTRUCT) };
                style.styleNew |= WS_EX_LAYERED;
            }
        },
        WM_NCDESTROY => {
            frames().remove(&(hwnd as isize));
            // SAFETY: removes the subclass this proc was installed with.
            unsafe { RemoveWindowSubclass(hwnd, Some(frame_proc), SUBCLASS_ID) };
        },
        _ => {},
    }
    // SAFETY: forwards the message to the next proc in the subclass chain.
    unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
}

fn sizing_edge(wparam: WPARAM) -> Option<SizingEdge> {
    Some(match u32::try_from(wparam).ok()? {
        WMSZ_LEFT => SizingEdge::Left,
        WMSZ_RIGHT => SizingEdge::Right,
        WMSZ_TOP => SizingEdge::Top,
        WMSZ_TOPLEFT => SizingEdge::TopLeft,
        WMSZ_TOPRIGHT => SizingEdge::TopRight,
        WMSZ_BOTTOM => SizingEdge::Bottom,
        WMSZ_BOTTOMLEFT => SizingEdge::BottomLeft,
        WMSZ_BOTTOMRIGHT => SizingEdge::BottomRight,
        _ => return None,
    })
}

/// Outer minus client size: the title bar and borders.
fn non_client_size(hwnd: HWND) -> (i32, i32) {
    let mut outer = RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    let mut client = outer;
    // SAFETY: `hwnd` is the window being sized; both RECTs are valid
    // out-pointers for the duration of the calls.
    let ok = unsafe {
        GetWindowRect(hwnd, &raw mut outer) != 0 && GetClientRect(hwnd, &raw mut client) != 0
    };
    if !ok {
        return (0, 0);
    }
    (
        (outer.right - outer.left) - (client.right - client.left),
        (outer.bottom - outer.top) - (client.bottom - client.top),
    )
}

pub(super) fn set_opacity(win: &Window, opacity: f64) -> Result<(), String> {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let alpha = (opacity * 255.0).round() as u8;
    let layered = alpha < u8::MAX;
    update(win, |state| state.alpha = layered.then_some(alpha))?;
    let hwnd = win.hwnd() as HWND;
    // SAFETY: `hwnd` is the live window owned by `win`; reading and
    // writing its extended style is the documented way to toggle layering.
    unsafe {
        let ex_style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        let flag = isize::try_from(WS_EX_LAYERED).unwrap_or(0);
        let next = if layered {
            ex_style | flag
        } else {
            ex_style & !flag
        };
        if next != ex_style {
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, next);
        }
        if layered && SetLayeredWindowAttributes(hwnd, 0, alpha, LWA_ALPHA) == 0 {
            return Err("setOpacity: SetLayeredWindowAttributes failed".to_string());
        }
    }
    Ok(())
}

pub(super) fn set_aspect_ratio(win: &Window, ratio: Option<f64>) -> Result<(), String> {
    update(win, |state| state.aspect = ratio).map_err(|error| format!("setAspectRatio: {error}"))
}

pub(super) fn set_movable(win: &Window, movable: bool) -> Result<(), String> {
    update(win, |state| state.immovable = !movable).map_err(|error| format!("setMovable: {error}"))
}

/// Windows has a single topmost band: every level maps to it.
pub(super) fn set_top_level(_win: &Window, _level: TopLevel) {}

/// No separate "simple" mode: borderless fullscreen never switches spaces.
pub(super) fn set_simple_fullscreen(win: &Window, fullscreen: bool) {
    win.set_fullscreen(fullscreen.then_some(Fullscreen::Borderless(None)));
}
