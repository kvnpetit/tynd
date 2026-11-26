//! System appearance preferences beyond light/dark: accent color, high
//! contrast, reduced motion, reduced transparency.
//!
//! - Windows: `HKCU\…\DWM\AccentColor` (falls back to
//!   `DwmGetColorizationColor`), `SPI_GETHIGHCONTRAST`,
//!   `SPI_GETCLIENTAREAANIMATION`, `HKCU\…\Themes\Personalize\EnableTransparency`.
//! - macOS: `NSColor.controlAccentColor` and the `NSWorkspace`
//!   `accessibilityDisplayShould*` flags.
//! - Linux: no desktop-neutral source, every field is `null`.

use serde_json::{json, Value};

/// One snapshot; `None` means "this OS does not expose it".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) struct Preferences {
    /// `0xRRGGBB`.
    pub(super) accent_color: Option<u32>,
    pub(super) high_contrast: Option<bool>,
    pub(super) reduced_motion: Option<bool>,
    pub(super) reduced_transparency: Option<bool>,
}

impl Preferences {
    pub(super) fn to_json(self, mode: &str) -> Value {
        json!({
            "mode": mode,
            "accentColor": self.accent_color.map(hex_color),
            "highContrast": self.high_contrast,
            "reducedMotion": self.reduced_motion,
            "reducedTransparency": self.reduced_transparency,
        })
    }
}

/// `0xRRGGBB` → `"#rrggbb"`.
pub(super) fn hex_color(rgb: u32) -> String {
    format!("#{:06x}", rgb & 0x00ff_ffff)
}

#[cfg(target_os = "windows")]
pub(super) fn read() -> Preferences {
    win::read()
}

#[cfg(target_os = "macos")]
pub(super) fn read() -> Preferences {
    mac::read()
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
pub(super) fn read() -> Preferences {
    Preferences::default()
}

#[cfg(target_os = "windows")]
mod win {
    use std::ffi::c_void;

    use windows_sys::Win32::{
        Graphics::Dwm::DwmGetColorizationColor,
        System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD},
        UI::WindowsAndMessaging::{
            SystemParametersInfoW, SPI_GETCLIENTAREAANIMATION, SPI_GETHIGHCONTRAST,
        },
    };

    use super::Preferences;

    /// `HIGHCONTRASTW` (WinUser.h); declared here to avoid pulling the
    /// `Win32_UI_Accessibility` feature for one struct.
    #[repr(C)]
    struct HighContrast {
        cb_size: u32,
        dw_flags: u32,
        default_scheme: *mut u16,
    }
    const HCF_HIGHCONTRASTON: u32 = 1;

    pub(super) fn read() -> Preferences {
        Preferences {
            accent_color: accent_color(),
            high_contrast: high_contrast(),
            reduced_motion: client_area_animation().map(|enabled| !enabled),
            reduced_transparency: reg_dword(
                "Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize",
                "EnableTransparency",
            )
            .map(|enabled| enabled == 0),
        }
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain([0]).collect()
    }

    fn reg_dword(subkey: &str, value: &str) -> Option<u32> {
        let subkey = wide(subkey);
        let value = wide(value);
        let mut data: u32 = 0;
        let mut size = size_of::<u32>() as u32;
        // SAFETY: NUL-terminated key/value names; `data`/`size` describe a
        // writable DWORD; RRF_RT_REG_DWORD rejects any other value type.
        let status = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                subkey.as_ptr(),
                value.as_ptr(),
                RRF_RT_REG_DWORD,
                std::ptr::null_mut(),
                std::ptr::addr_of_mut!(data).cast::<c_void>(),
                &raw mut size,
            )
        };
        (status == 0).then_some(data)
    }

    /// `AccentColor` is stored as `0xAABBGGRR`.
    fn accent_color() -> Option<u32> {
        if let Some(abgr) = reg_dword("Software\\Microsoft\\Windows\\DWM", "AccentColor") {
            return Some(super::abgr_to_rgb(abgr));
        }
        let mut argb: u32 = 0;
        let mut opaque = 0;
        // SAFETY: both out pointers reference live locals.
        let hr = unsafe { DwmGetColorizationColor(&raw mut argb, &raw mut opaque) };
        (hr >= 0).then_some(argb & 0x00ff_ffff)
    }

    fn high_contrast() -> Option<bool> {
        let mut info = HighContrast {
            cb_size: size_of::<HighContrast>() as u32,
            dw_flags: 0,
            default_scheme: std::ptr::null_mut(),
        };
        // SAFETY: SPI_GETHIGHCONTRAST fills a HIGHCONTRASTW whose size is
        // passed both in `cbSize` and `uiParam`.
        let ok = unsafe {
            SystemParametersInfoW(
                SPI_GETHIGHCONTRAST,
                info.cb_size,
                std::ptr::addr_of_mut!(info).cast::<c_void>(),
                0,
            )
        } != 0;
        ok.then_some(info.dw_flags & HCF_HIGHCONTRASTON != 0)
    }

    fn client_area_animation() -> Option<bool> {
        let mut enabled: i32 = 0;
        // SAFETY: SPI_GETCLIENTAREAANIMATION writes a BOOL.
        let ok = unsafe {
            SystemParametersInfoW(
                SPI_GETCLIENTAREAANIMATION,
                0,
                std::ptr::addr_of_mut!(enabled).cast::<c_void>(),
                0,
            )
        } != 0;
        ok.then_some(enabled != 0)
    }
}

#[cfg(target_os = "macos")]
mod mac {
    use objc2_app_kit::{NSColor, NSColorSpace, NSWorkspace};

    use super::Preferences;

    pub(super) fn read() -> Preferences {
        let workspace = NSWorkspace::sharedWorkspace();
        Preferences {
            accent_color: accent_color(),
            high_contrast: Some(workspace.accessibilityDisplayShouldIncreaseContrast()),
            reduced_motion: Some(workspace.accessibilityDisplayShouldReduceMotion()),
            reduced_transparency: Some(workspace.accessibilityDisplayShouldReduceTransparency()),
        }
    }

    fn accent_color() -> Option<u32> {
        let color =
            NSColor::controlAccentColor().colorUsingColorSpace(&NSColorSpace::sRGBColorSpace())?;
        let channel = |value: f64| -> u32 { (value.clamp(0.0, 1.0) * 255.0).round() as u32 };
        Some(
            (channel(color.redComponent()) << 16)
                | (channel(color.greenComponent()) << 8)
                | channel(color.blueComponent()),
        )
    }
}

/// Registry `0xAABBGGRR` → `0xRRGGBB`.
#[cfg_attr(
    not(any(test, target_os = "windows")),
    allow(dead_code, reason = "Windows-only byte order")
)]
fn abgr_to_rgb(abgr: u32) -> u32 {
    let r = abgr & 0xff;
    let g = (abgr >> 8) & 0xff;
    let b = (abgr >> 16) & 0xff;
    (r << 16) | (g << 8) | b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors_are_lowercase_hex() {
        assert_eq!(hex_color(0x0000_78d4), "#0078d4");
        assert_eq!(hex_color(0xff12_3456), "#123456");
        assert_eq!(abgr_to_rgb(0xffd4_7800), 0x0000_78d4);
    }

    #[test]
    fn snapshot_serializes_unknown_fields_as_null() {
        let value = Preferences::default().to_json("dark");
        assert_eq!(value["mode"], "dark");
        assert!(value["accentColor"].is_null());
        assert!(value["reducedMotion"].is_null());
        let value = Preferences {
            accent_color: Some(0x0000_ff00),
            high_contrast: Some(true),
            ..Preferences::default()
        }
        .to_json("light");
        assert_eq!(value["accentColor"], "#00ff00");
        assert_eq!(value["highContrast"], true);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_reads_the_accessibility_flags() {
        let prefs = read();
        assert!(prefs.high_contrast.is_some());
        assert!(prefs.reduced_motion.is_some());
    }
}
