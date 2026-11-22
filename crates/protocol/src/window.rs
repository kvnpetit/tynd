use serde::Deserialize;
use specta::Type;

/// GPU vendor pin for [`GpuPreference::Vendor`]. Free-form strings are tolerated
/// at the JSON layer; unknown vendors degrade to `auto` in the host.
#[derive(Debug, Deserialize, Clone, PartialEq, Eq, Type)]
#[serde(rename_all = "camelCase")]
pub struct GpuVendor {
    /// `"nvidia"` | `"amd"` | `"intel"` (lowercase, matched case-insensitively).
    pub vendor: String,
}

/// Multi-GPU adapter selection.
#[derive(Debug, Deserialize, Clone, PartialEq, Eq, Type)]
#[serde(rename_all = "camelCase", untagged)]
#[non_exhaustive]
pub enum GpuPreference {
    /// `"auto" | "primary" | "discrete"` string literals.
    Named(String),
    /// `{ vendor: "nvidia" | "amd" | "intel" }`.
    Vendor(GpuVendor),
}

/// Color-space hint for the OS swap chain.
#[derive(Debug, Deserialize, Clone, PartialEq, Eq, Type)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum ColorSpace {
    Srgb,
    P3,
    Rec2020,
    Linear,
}

/// Refresh-rate hint.
#[derive(Debug, Deserialize, Clone, PartialEq, Type)]
#[serde(untagged)]
#[non_exhaustive]
pub enum PreferredFps {
    /// `"monitor"` literal.
    Auto(String),
    /// Explicit cap in Hz.
    Hz(u32),
}

/// Hardware-acceleration tier.
#[derive(Debug, Deserialize, Clone, PartialEq, Eq, Type)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum HardwareAcceleration {
    Default,
    HighPerformance,
    LowPower,
}

/// macOS title bar style. Ignored on Windows and Linux.
#[derive(Debug, Deserialize, Clone, Copy, PartialEq, Eq, Type)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum TitleBarStyle {
    /// Standard opaque title bar.
    Visible,
    /// Transparent title bar; the title and traffic lights stay visible.
    Transparent,
    /// Transparent title bar, hidden title, page content drawn under the
    /// traffic lights (`hiddenInset`-style).
    Overlay,
}

/// Window theme override.
#[derive(Debug, Deserialize, Clone, Copy, PartialEq, Eq, Type)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum WindowTheme {
    Light,
    Dark,
    /// Follow the OS theme (default).
    System,
}

/// RGBA color, each channel `0..=255`; `a` defaults to `255` (opaque).
#[derive(Debug, Deserialize, Clone, Copy, PartialEq, Eq, Type)]
pub struct WindowColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: Option<u8>,
}

/// Window configuration provided by the TypeScript backend via `app.start()`.
#[derive(Debug, Deserialize, Default, Clone, Type, bon::Builder)]
#[serde(rename_all = "camelCase")]
pub struct WindowConfig {
    pub title: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub min_width: Option<u32>,
    pub min_height: Option<u32>,
    pub max_width: Option<u32>,
    pub max_height: Option<u32>,
    pub resizable: Option<bool>,
    pub decorations: Option<bool>,
    pub transparent: Option<bool>,
    pub always_on_top: Option<bool>,
    pub center: Option<bool>,
    pub fullscreen: Option<bool>,
    pub maximized: Option<bool>,
    /// `false` keeps the window hidden after the first page load until the
    /// frontend calls `tyndWindow.show()` (splash-screen pattern).
    pub visible: Option<bool>,
    /// Start in kiosk mode (borderless fullscreen, pinned above other windows).
    pub kiosk: Option<bool>,
    /// macOS title bar style.
    pub title_bar_style: Option<TitleBarStyle>,
    /// Initial outer position in logical pixels. Needs both `x` and `y`;
    /// `center: true` wins.
    pub x: Option<f64>,
    pub y: Option<f64>,
    /// Theme override; unset follows the OS.
    pub theme: Option<WindowTheme>,
    /// Hide the window from the taskbar (Windows / Linux; ignored on macOS).
    pub skip_taskbar: Option<bool>,
    /// Native background shown before the page paints and through
    /// transparent HTML.
    pub background_color: Option<WindowColor>,
    /// Custom User-Agent string set on every webview request.
    pub user_agent: Option<String>,
    /// Use a non-persistent webview data store for this window.
    pub incognito: Option<bool>,
    /// `window.open()` / `target="_blank"` policy: `"deny"`, `"external"` or
    /// `"allow"` (default). Unknown values are rejected by the host.
    pub new_window: Option<String>,
    /// Download policy: `"allow"` or `"deny"`. Unset = allow while the window
    /// shows app-local content, deny while it shows remote content. Unknown
    /// values are rejected by the host.
    pub downloads: Option<String>,
    /// Webview proxy: `http://host[:port]` or `socks5://host:port`, no credentials.
    pub proxy: Option<String>,
    /// Web permission decisions keyed by kind (`camera`, `microphone`,
    /// `geolocation`, ...): `"allow"`, `"deny"` or `"prompt"`. Unset kinds
    /// prompt for app content and are denied for remote content. Unknown keys
    /// and values are rejected by the host.
    pub permissions: Option<std::collections::BTreeMap<String, String>>,
    /// Enable WebGPU.
    pub webgpu: Option<bool>,
    /// Wires to Chromium GPU flags + Linux DRI / NV-PRIME env vars.
    pub hardware_acceleration: Option<HardwareAcceleration>,
    /// Refresh-rate hint.
    pub preferred_fps: Option<PreferredFps>,
    /// Auto-reload the webview when the GPU process dies.
    pub context_lost_recovery: Option<bool>,
    /// Wide-gamut / HDR opt-in.
    pub color_space: Option<ColorSpace>,
    pub hdr: Option<bool>,
    /// Variable-refresh-rate.
    pub vrr: Option<bool>,
    /// Multi-GPU adapter pin for hybrid-graphics workstations.
    pub gpu: Option<GpuPreference>,
}

#[cfg(test)]
mod new_window_proxy_tests {
    use super::WindowConfig;

    #[test]
    fn new_window_and_proxy_default_to_none() {
        let config: WindowConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(config.new_window, None);
        assert_eq!(config.downloads, None);
        assert_eq!(config.proxy, None);
    }

    #[test]
    fn new_window_and_proxy_use_camel_case_keys() {
        let config: WindowConfig = serde_json::from_str(
            r#"{"newWindow":"deny","proxy":"socks5://127.0.0.1:1080","title":"t"}"#,
        )
        .unwrap();
        assert_eq!(config.new_window.as_deref(), Some("deny"));
        assert_eq!(config.proxy.as_deref(), Some("socks5://127.0.0.1:1080"));
    }

    #[test]
    fn visibility_kiosk_and_title_bar_style_use_camel_case_keys() {
        let config: WindowConfig =
            serde_json::from_str(r#"{"visible":false,"kiosk":true,"titleBarStyle":"overlay"}"#)
                .unwrap();
        assert_eq!(config.visible, Some(false));
        assert_eq!(config.kiosk, Some(true));
        assert_eq!(config.title_bar_style, Some(super::TitleBarStyle::Overlay));
        assert!(
            serde_json::from_str::<WindowConfig>(r#"{"titleBarStyle":"hiddenInset"}"#).is_err()
        );
    }

    #[test]
    fn position_theme_taskbar_and_background_use_camel_case_keys() {
        let config: WindowConfig = serde_json::from_str(
            r#"{"x":-10.5,"y":20,"theme":"dark","skipTaskbar":true,"backgroundColor":{"r":1,"g":2,"b":3}}"#,
        )
        .unwrap();
        assert_eq!((config.x, config.y), (Some(-10.5), Some(20.0)));
        assert_eq!(config.theme, Some(super::WindowTheme::Dark));
        assert_eq!(config.skip_taskbar, Some(true));
        assert_eq!(
            config.background_color,
            Some(super::WindowColor {
                r: 1,
                g: 2,
                b: 3,
                a: None
            })
        );
        assert!(serde_json::from_str::<WindowConfig>(
            r#"{"backgroundColor":{"r":256,"g":0,"b":0}}"#
        )
        .is_err());
        assert!(serde_json::from_str::<WindowConfig>(r#"{"theme":"auto"}"#).is_err());
    }

    #[test]
    fn permissions_is_a_kind_to_decision_map() {
        let config: WindowConfig =
            serde_json::from_str(r#"{"permissions":{"camera":"allow"}}"#).unwrap();
        let permissions = config.permissions.unwrap();
        assert_eq!(permissions.get("camera").map(String::as_str), Some("allow"));
    }

    #[test]
    fn downloads_uses_its_camel_case_key() {
        let config: WindowConfig = serde_json::from_str(r#"{"downloads":"deny"}"#).unwrap();
        assert_eq!(config.downloads.as_deref(), Some("deny"));
    }
}
