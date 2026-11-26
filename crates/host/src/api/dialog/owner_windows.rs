//! Owner window of a dialog on Windows: the app's foreground top-level
//! window. IPC calls run on pool threads that cannot reach the tao windows,
//! but the window the user just interacted with to trigger a dialog is the
//! process's foreground window, and an HWND is usable as an owner from any
//! thread.

use std::num::NonZeroIsize;

use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, RawWindowHandle,
    Win32WindowHandle, WindowHandle,
};
use windows_sys::Win32::{
    Foundation::HWND,
    System::Threading::GetCurrentProcessId,
    UI::WindowsAndMessaging::{
        GetAncestor, GetForegroundWindow, GetWindowThreadProcessId, GA_ROOT,
    },
};

/// Root of the foreground window when it belongs to this process, else null
/// (the dialog is then unowned).
pub(super) fn foreground_window() -> HWND {
    // SAFETY: no arguments; returns a possibly-null HWND.
    let foreground = unsafe { GetForegroundWindow() };
    if foreground.is_null() {
        return std::ptr::null_mut();
    }
    let mut pid = 0u32;
    // SAFETY: `foreground` is a window handle (possibly stale: the call then
    // returns 0 and leaves `pid` at 0); `pid` is a valid out pointer.
    unsafe { GetWindowThreadProcessId(foreground, &raw mut pid) };
    // SAFETY: no arguments.
    if pid != unsafe { GetCurrentProcessId() } {
        return std::ptr::null_mut();
    }
    // SAFETY: `foreground` is a window handle; a stale one yields null.
    unsafe { GetAncestor(foreground, GA_ROOT) }
}

/// `raw-window-handle` view of an HWND, for `rfd`'s `set_parent`.
#[derive(Debug, Clone, Copy)]
pub(super) struct OwnerHandle(NonZeroIsize);

impl OwnerHandle {
    /// The app's foreground window, if any.
    pub(super) fn foreground() -> Option<Self> {
        NonZeroIsize::new(foreground_window() as isize).map(Self)
    }
}

impl HasWindowHandle for OwnerHandle {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        let raw = RawWindowHandle::Win32(Win32WindowHandle::new(self.0));
        // SAFETY: the HWND came from the window manager; `rfd` only passes it
        // as the dialog owner, and a window closed meanwhile makes the dialog
        // unowned rather than unsound.
        Ok(unsafe { WindowHandle::borrow_raw(raw) })
    }
}

impl HasDisplayHandle for OwnerHandle {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        Ok(DisplayHandle::windows())
    }
}
