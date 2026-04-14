use tao::window::Window;

pub(super) fn set_skip_taskbar(window: &Window, skip: bool) {
    #[cfg(target_os = "windows")]
    {
        use tao::platform::windows::WindowExtWindows;
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            GetWindowLongW, SetWindowLongW, SetWindowPos,
            GWL_EXSTYLE, WS_EX_TOOLWINDOW,
            SWP_FRAMECHANGED, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER,
        };
        let hwnd = window.hwnd() as _;
        unsafe {
            let mut ex_style = GetWindowLongW(hwnd, GWL_EXSTYLE);
            if skip { ex_style |=  WS_EX_TOOLWINDOW as i32; }
            else    { ex_style &= !(WS_EX_TOOLWINDOW as i32); }
            SetWindowLongW(hwnd, GWL_EXSTYLE, ex_style);
            SetWindowPos(hwnd, 0, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_FRAMECHANGED);
        }
    }
    #[cfg(target_os = "macos")]
    {
        use objc::{msg_send, sel, sel_impl};
        // NSApplicationActivationPolicyAccessory = 1 (no dock), Regular = 0
        let app: *mut objc::runtime::Object = unsafe {
            msg_send![objc::class!(NSApplication), sharedApplication]
        };
        let policy: isize = if skip { 1 } else { 0 };
        unsafe { let _: () = msg_send![app, setActivationPolicy: policy]; }
        let _ = window;
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let _ = (window, skip);
}

pub(super) fn request_user_attention(window: &Window, critical: bool) {
    #[cfg(target_os = "windows")]
    {
        use tao::platform::windows::WindowExtWindows;
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            FlashWindowEx, FLASHWINFO, FLASHW_ALL, FLASHW_TIMERNOFG, FLASHW_TRAY,
        };
        let hwnd  = window.hwnd() as _;
        let flags = if critical { FLASHW_ALL | FLASHW_TIMERNOFG } else { FLASHW_TRAY };
        let info  = FLASHWINFO {
            cbSize:    std::mem::size_of::<FLASHWINFO>() as u32,
            hwnd,
            dwFlags:   flags,
            uCount:    if critical { u32::MAX } else { 3 },
            dwTimeout: 0,
        };
        unsafe { FlashWindowEx(&info); }
    }
    #[cfg(target_os = "macos")]
    {
        use objc::{msg_send, sel, sel_impl};
        // NSRequestUserAttentionType: 0 = NSCriticalRequest, 1 = NSInformationalRequest
        let app: *mut objc::runtime::Object = unsafe {
            msg_send![objc::class!(NSApplication), sharedApplication]
        };
        let request_type: isize = if critical { 0 } else { 1 };
        unsafe { let _: isize = msg_send![app, requestUserAttention: request_type]; }
        let _ = window;
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let _ = (window, critical);
}

pub(super) fn set_content_protected(window: &Window, protected: bool) {
    #[cfg(target_os = "windows")]
    {
        use tao::platform::windows::WindowExtWindows;
        use windows_sys::Win32::UI::WindowsAndMessaging::SetWindowDisplayAffinity;
        // WDA_NONE = 0, WDA_MONITOR = 1
        unsafe { SetWindowDisplayAffinity(window.hwnd() as _, if protected { 1 } else { 0 }); }
    }
    #[cfg(target_os = "macos")]
    {
        use objc::{msg_send, sel, sel_impl};
        use tao::platform::macos::WindowExtMacOS;
        let ns_window = window.ns_window() as *mut objc::runtime::Object;
        // NSWindowSharingNone = 0, NSWindowSharingReadOnly = 1, NSWindowSharingReadWrite = 3
        let sharing_type: isize = if protected { 0 } else { 3 };
        unsafe { let _: () = msg_send![ns_window, setSharingType: sharing_type]; }
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let _ = (window, protected);
}

pub(super) fn set_always_on_bottom(window: &Window, on: bool) {
    #[cfg(target_os = "windows")]
    {
        use tao::platform::windows::WindowExtWindows;
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            SetWindowPos, HWND_BOTTOM, HWND_NOTOPMOST,
            SWP_NOMOVE, SWP_NOSIZE, SWP_NOACTIVATE,
        };
        let insert_after: isize = if on { HWND_BOTTOM } else { HWND_NOTOPMOST };
        unsafe {
            SetWindowPos(
                window.hwnd() as _,
                insert_after, 0, 0, 0, 0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
        }
    }
    #[cfg(target_os = "macos")]
    {
        use objc::{msg_send, sel, sel_impl};
        use tao::platform::macos::WindowExtMacOS;
        let ns_window = window.ns_window() as *mut objc::runtime::Object;
        // NSNormalWindowLevel = 0; below-normal = -1
        let level: isize = if on { -1 } else { 0 };
        unsafe { let _: () = msg_send![ns_window, setLevel: level]; }
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let _ = (window, on);
}

pub(super) fn set_progress_bar(window: &Window, progress: Option<f64>) {
    #[cfg(target_os = "windows")]
    {
        use tao::platform::windows::WindowExtWindows;
        set_progress_bar_win(window.hwnd() as isize, progress);
    }
    #[cfg(target_os = "macos")]
    {
        use objc::{msg_send, sel, sel_impl};
        let label = match progress {
            Some(p) => format!("{:.0}%", p.clamp(0.0, 1.0) * 100.0),
            None    => String::new(),
        };
        let c_str = std::ffi::CString::new(label).unwrap();
        unsafe {
            let app: *mut objc::runtime::Object =
                msg_send![objc::class!(NSApplication), sharedApplication];
            let dock_tile: *mut objc::runtime::Object = msg_send![app, dockTile];
            let ns_str: *mut objc::runtime::Object = msg_send![
                objc::class!(NSString),
                stringWithUTF8String: c_str.as_ptr()
            ];
            let _: () = msg_send![dock_tile, setBadgeLabel: ns_str];
        }
        let _ = window;
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let _ = (window, progress);
}

#[cfg(target_os = "windows")]
fn set_progress_bar_win(hwnd: isize, progress: Option<f64>) {
    use windows_sys::Win32::System::Com::{CoCreateInstance, CLSCTX_ALL};
    use windows_sys::core::GUID;

    const CLSID: GUID = GUID {
        data1: 0x56FDF344, data2: 0xFD6D, data3: 0x11D0,
        data4: [0x95, 0x8A, 0x00, 0x60, 0x97, 0xC9, 0xA0, 0x90],
    };
    const IID: GUID = GUID {
        data1: 0xEA1AFB91, data2: 0x9E28, data3: 0x4B86,
        data4: [0x90, 0xE9, 0x9E, 0x9F, 0x8A, 0x5E, 0xEF, 0xAF],
    };

    unsafe {
        let mut ptr: *mut std::ffi::c_void = std::ptr::null_mut();
        if CoCreateInstance(&CLSID, std::ptr::null_mut(), CLSCTX_ALL, &IID, &mut ptr) < 0
            || ptr.is_null()
        {
            return;
        }

        // ITaskbarList3 vtable:
        // 0=QI 1=AddRef 2=Release 3=HrInit 4=AddTab 5=DeleteTab
        // 6=ActivateTab 7=SetActiveAlt 8=MarkFullscreen
        // 9=SetProgressValue 10=SetProgressState
        let vtbl = *(ptr as *mut *const *const ());

        let hr_init: unsafe extern "system" fn(*mut std::ffi::c_void) -> i32 =
            std::mem::transmute(*vtbl.add(3));
        hr_init(ptr);

        let set_state: unsafe extern "system" fn(*mut std::ffi::c_void, isize, u32) -> i32 =
            std::mem::transmute(*vtbl.add(10));
        let set_value: unsafe extern "system" fn(*mut std::ffi::c_void, isize, u64, u64) -> i32 =
            std::mem::transmute(*vtbl.add(9));

        match progress {
            Some(p) => {
                set_state(ptr, hwnd, 0x2); // TBPF_NORMAL
                set_value(ptr, hwnd, (p.clamp(0.0, 1.0) * 100.0) as u64, 100);
            }
            None => {
                set_state(ptr, hwnd, 0x0); // TBPF_NOPROGRESS
            }
        }

        let release: unsafe extern "system" fn(*mut std::ffi::c_void) -> u32 =
            std::mem::transmute(*vtbl.add(2));
        release(ptr);
    }
}

/// Set the app's dock badge (macOS) — e.g. unread message count.
/// Pass `None` or empty string to clear. No-op on Windows/Linux.
pub(super) fn set_badge_count(_window: &tao::window::Window, count: Option<i64>) {
    #[cfg(target_os = "macos")]
    {
        use objc::{msg_send, sel, sel_impl};
        let app: *mut objc::runtime::Object = unsafe {
            msg_send![objc::class!(NSApplication), sharedApplication]
        };
        let tile: *mut objc::runtime::Object = unsafe { msg_send![app, dockTile] };

        let label: *mut objc::runtime::Object = match count {
            Some(n) if n > 0 => {
                let s = n.to_string();
                let c_string = std::ffi::CString::new(s).unwrap();
                unsafe {
                    msg_send![objc::class!(NSString),
                        stringWithUTF8String: c_string.as_ptr()]
                }
            }
            _ => std::ptr::null_mut(),
        };
        unsafe { let _: () = msg_send![tile, setBadgeLabel: label]; }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = count;
}
