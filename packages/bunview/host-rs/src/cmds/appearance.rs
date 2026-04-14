use tao::window::Window;
use wry::WebView;

pub(super) fn set_enabled(window: &Window, enabled: bool) {
    #[cfg(target_os = "windows")]
    {
        // EnableWindow is not exported in windows-sys 0.52 — declare directly.
        extern "system" { fn EnableWindow(hwnd: isize, benable: i32) -> i32; }
        use tao::platform::windows::WindowExtWindows;
        unsafe { EnableWindow(window.hwnd() as _, enabled as i32); }
    }
    #[cfg(target_os = "macos")]
    {
        use objc::{msg_send, sel, sel_impl};
        use tao::platform::macos::WindowExtMacOS;
        let ns_window = window.ns_window() as *mut objc::runtime::Object;
        unsafe { let _: () = msg_send![ns_window, setIgnoresMouseEvents: !enabled]; }
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let _ = (window, enabled);
}

pub(super) fn set_shadow(window: &Window, shadow: bool) {
    #[cfg(target_os = "windows")]
    {
        use tao::platform::windows::WindowExtWindows;
        use windows_sys::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_NCRENDERING_POLICY};
        // DWMNCRP_DISABLED = 1, DWMNCRP_ENABLED = 2
        let policy: u32 = if shadow { 2 } else { 1 };
        unsafe {
            let _ = DwmSetWindowAttribute(
                window.hwnd() as _,
                DWMWA_NCRENDERING_POLICY as u32,
                &policy as *const _ as _,
                std::mem::size_of::<u32>() as u32,
            );
        }
    }
    #[cfg(target_os = "macos")]
    {
        use objc::{msg_send, sel, sel_impl};
        use tao::platform::macos::WindowExtMacOS;
        let ns_window = window.ns_window() as *mut objc::runtime::Object;
        unsafe { let _: () = msg_send![ns_window, setHasShadow: shadow]; }
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let _ = (window, shadow);
}

pub(super) fn set_title_bar_style(window: &Window, style: &str) {
    #[cfg(target_os = "macos")]
    {
        use objc::{msg_send, sel, sel_impl};
        use tao::platform::macos::WindowExtMacOS;
        let ns_window = window.ns_window() as *mut objc::runtime::Object;
        // NSWindowStyleMaskFullSizeContentView = 1 << 15
        const FULL_SIZE_CONTENT: usize = 1 << 15;
        unsafe {
            let mask: usize = msg_send![ns_window, styleMask];
            match style {
                "hidden" => {
                    let _: () = msg_send![ns_window, setTitlebarAppearsTransparent: true];
                    let _: () = msg_send![ns_window, setStyleMask: mask | FULL_SIZE_CONTENT];
                    let _: () = msg_send![ns_window, setTitleVisibility: 1_isize]; // NSWindowTitleHidden
                }
                "transparent" | "overlay" => {
                    let _: () = msg_send![ns_window, setTitlebarAppearsTransparent: true];
                    let _: () = msg_send![ns_window, setStyleMask: mask | FULL_SIZE_CONTENT];
                    let _: () = msg_send![ns_window, setTitleVisibility: 0_isize]; // NSWindowTitleVisible
                }
                _ => { // "visible"
                    let _: () = msg_send![ns_window, setTitlebarAppearsTransparent: false];
                    let _: () = msg_send![ns_window, setStyleMask: mask & !FULL_SIZE_CONTENT];
                    let _: () = msg_send![ns_window, setTitleVisibility: 0_isize];
                }
            }
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = (window, style);
}

pub(super) fn set_vibrancy_impl(window: &Window, effect: &str) {
    #[cfg(target_os = "windows")]
    {
        use window_vibrancy::{apply_acrylic, apply_mica};
        match effect {
            "mica" | "tabbed" => { let _ = apply_mica(window, None); }
            "acrylic"         => { let _ = apply_acrylic(window, Some((0, 0, 0, 50))); }
            _ => {}
        }
    }
    #[cfg(target_os = "macos")]
    {
        use window_vibrancy::{apply_vibrancy, NSVisualEffectMaterial};
        let material = match effect {
            "light"   => NSVisualEffectMaterial::Light,
            "dark"    => NSVisualEffectMaterial::Dark,
            "acrylic" => NSVisualEffectMaterial::HudWindow,
            _         => NSVisualEffectMaterial::AppearanceBased,
        };
        let _ = apply_vibrancy(window, material, None, None);
    }
    #[cfg(target_os = "linux")]
    let _ = (window, effect);
}

pub(super) fn set_buttons(window: &Window, minimize: bool, maximize: bool, close: bool) {
    #[cfg(target_os = "windows")]
    {
        use tao::platform::windows::WindowExtWindows;
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            EnableMenuItem, GetSystemMenu, GetWindowLongW, SetWindowLongW, SetWindowPos,
            GWL_STYLE, MF_BYCOMMAND, MF_ENABLED, MF_GRAYED, SC_CLOSE,
            SWP_FRAMECHANGED, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER,
            WS_MAXIMIZEBOX, WS_MINIMIZEBOX,
        };
        let hwnd = window.hwnd() as _;
        unsafe {
            let mut style = GetWindowLongW(hwnd, GWL_STYLE);
            if minimize { style |= WS_MINIMIZEBOX as i32; } else { style &= !(WS_MINIMIZEBOX as i32); }
            if maximize { style |= WS_MAXIMIZEBOX as i32; } else { style &= !(WS_MAXIMIZEBOX as i32); }
            SetWindowLongW(hwnd, GWL_STYLE, style);
            let hmenu = GetSystemMenu(hwnd, 0);
            if hmenu != 0 {
                EnableMenuItem(hmenu, SC_CLOSE, MF_BYCOMMAND | if close { MF_ENABLED } else { MF_GRAYED });
            }
            SetWindowPos(hwnd, 0, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_FRAMECHANGED);
        }
    }
    #[cfg(target_os = "macos")]
    {
        use objc::{msg_send, sel, sel_impl};
        use tao::platform::macos::WindowExtMacOS;
        let ns_window = window.ns_window() as *mut objc::runtime::Object;
        // NSWindowCloseButton=0, NSWindowMiniaturizeButton=1, NSWindowZoomButton=2
        let buttons = [(0usize, close), (1, minimize), (2, maximize)];
        unsafe {
            for (idx, show) in buttons {
                let btn: *mut objc::runtime::Object =
                    msg_send![ns_window, standardWindowButton: idx];
                if !btn.is_null() {
                    let _: () = msg_send![btn, setHidden: !show];
                }
            }
        }
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let _ = (window, minimize, maximize, close);
}

pub(super) fn set_background_color(webview: &WebView, r: u8, g: u8, b: u8, a: u8) {
    let alpha  = a as f32 / 255.0;
    let script = format!(
        "var s=document.getElementById('__bv_bg');\
         if(!s){{s=document.createElement('style');s.id='__bv_bg';document.head.appendChild(s);}}\
         s.textContent='html,body{{background:rgba({r},{g},{b},{alpha:.4}) !important}}'",
    );
    let _ = webview.evaluate_script(&script);
}
