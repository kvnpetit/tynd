//! `NSWindow` side of the frame controls (window levels, alpha, content
//! aspect ratio, movability). Untested on CI builds made from Windows.

use objc2::{msg_send, runtime::AnyObject, Encode, Encoding};
use tao::{platform::macos::WindowExtMacOS, window::Window};

use super::TopLevel;

/// tao cannot start a resize drag on macOS; `NSWindow` handles the edges of
/// resizable borderless windows itself.
pub(super) const SUPPORTS_EDGE_RESIZE: bool = false;

#[repr(C)]
struct NsSize {
    width: f64,
    height: f64,
}

// SAFETY: `NsSize` has the exact layout of `CGSize` (two `CGFloat`s).
unsafe impl Encode for NsSize {
    const ENCODING: Encoding = Encoding::Struct("CGSize", &[f64::ENCODING, f64::ENCODING]);
}

fn ns_window(win: &Window) -> *mut AnyObject {
    win.ns_window().cast()
}

pub(super) fn set_top_level(win: &Window, level: TopLevel) {
    // NSFloatingWindowLevel / NSStatusWindowLevel / NSScreenSaverWindowLevel.
    let level: isize = match level {
        TopLevel::Floating => 3,
        TopLevel::Status => 25,
        TopLevel::ScreenSaver => 1000,
    };
    // SAFETY: `ns_window` is the live NSWindow owned by `win`; called on
    // the main thread. `setLevel:` takes an NSInteger.
    unsafe {
        let _: () = msg_send![ns_window(win), setLevel: level];
    }
}

pub(super) fn set_opacity(win: &Window, opacity: f64) -> Result<(), String> {
    // SAFETY: as above; `setAlphaValue:` takes a CGFloat.
    unsafe {
        let _: () = msg_send![ns_window(win), setAlphaValue: opacity];
    }
    Ok(())
}

pub(super) fn set_aspect_ratio(win: &Window, ratio: Option<f64>) -> Result<(), String> {
    let window = ns_window(win);
    // SAFETY: as above; both selectors take an NSSize. Resetting the
    // resize increments to 1×1 is how AppKit clears an aspect ratio.
    unsafe {
        match ratio {
            Some(ratio) => {
                let size = NsSize {
                    width: ratio,
                    height: 1.0,
                };
                let _: () = msg_send![window, setContentAspectRatio: size];
            },
            None => {
                let size = NsSize {
                    width: 1.0,
                    height: 1.0,
                };
                let _: () = msg_send![window, setContentResizeIncrements: size];
            },
        }
    }
    Ok(())
}

pub(super) fn set_movable(win: &Window, movable: bool) -> Result<(), String> {
    // SAFETY: as above; `setMovable:` takes a BOOL.
    unsafe {
        let _: () = msg_send![ns_window(win), setMovable: movable];
    }
    Ok(())
}

/// Pre-Lion fullscreen: covers the screen without a new Space.
pub(super) fn set_simple_fullscreen(win: &Window, fullscreen: bool) {
    win.set_simple_fullscreen(fullscreen);
}
