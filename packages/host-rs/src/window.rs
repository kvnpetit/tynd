use serde::Deserialize;
use specta::Type;
use tao::{
    dpi::LogicalSize,
    event_loop::EventLoop,
    window::{Window, WindowBuilder},
};

/// Window configuration provided by the TypeScript backend via `app.start()`.
#[derive(Debug, Deserialize, Default, Clone, Type)]
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
    /// Custom User-Agent string set on every webview request.
    pub user_agent: Option<String>,
    /// Native blur/material effect applied to the window chrome.
    ///
    /// Accepted values and their platform coverage:
    /// - `"blur"` — Windows 10 acrylic-blur fallback (calls `apply_blur`)
    /// - `"acrylic"` — Windows 10/11 acrylic (frosted glass)
    /// - `"mica"` — Windows 11 Mica (desktop-wallpaper tint)
    /// - `"tabbed"` — Windows 11 Mica-Tabbed variant
    /// - `"vibrancy"` / `"sidebar"` / `"hud"` — macOS `NSVisualEffectView`
    ///
    /// Silently ignored (and logged) on platforms or OS versions that do not
    /// support the requested effect — vibrancy is optional polish, never a
    /// hard requirement.
    pub vibrancy: Option<String>,
}

/// Build a native window from a `WindowConfig`.
/// The window starts hidden — shown after the first page renders (no white flash).
pub fn build_window<T>(
    cfg: &WindowConfig,
    icon_path: Option<&str>,
    event_loop: &EventLoop<T>,
) -> Window {
    let mut wb = WindowBuilder::new()
        .with_title(cfg.title.as_deref().unwrap_or(""))
        .with_inner_size(LogicalSize::new(
            cfg.width.unwrap_or(1200),
            cfg.height.unwrap_or(800),
        ))
        .with_resizable(cfg.resizable.unwrap_or(true))
        .with_decorations(cfg.decorations.unwrap_or(true))
        .with_transparent(cfg.transparent.unwrap_or(false))
        .with_always_on_top(cfg.always_on_top.unwrap_or(false))
        .with_visible(false);

    if let (Some(min_w), Some(min_h)) = (cfg.min_width, cfg.min_height) {
        wb = wb.with_min_inner_size(LogicalSize::new(min_w, min_h));
    }

    if let (Some(max_w), Some(max_h)) = (cfg.max_width, cfg.max_height) {
        wb = wb.with_max_inner_size(LogicalSize::new(max_w, max_h));
    }

    if let Some(path) = icon_path {
        match crate::os::icon::load_tao(path) {
            Ok(icon) => {
                wb = wb.with_window_icon(Some(icon));
            },
            Err(e) => crate::tynd_log!("Window icon '{}': {e}", path),
        }
    }

    let window = match wb.build(event_loop) {
        Ok(w) => w,
        Err(e) => {
            crate::tynd_error!("Failed to create native window: {e}");
            std::process::exit(1);
        },
    };

    if cfg.maximized.unwrap_or(false) {
        window.set_maximized(true);
    }
    if cfg.fullscreen.unwrap_or(false) {
        window.set_fullscreen(Some(tao::window::Fullscreen::Borderless(None)));
    }

    // Apply native blur/material effect when requested.
    // Errors are logged and swallowed — vibrancy is cosmetic, never fatal.
    #[cfg(target_os = "macos")]
    if let Some(ref v) = cfg.vibrancy {
        use window_vibrancy::{apply_vibrancy, NSVisualEffectMaterial};
        let material = match v.as_str() {
            "sidebar" => NSVisualEffectMaterial::Sidebar,
            "hud"     => NSVisualEffectMaterial::HudWindow,
            _         => NSVisualEffectMaterial::UnderWindowBackground,
        };
        if let Err(e) = apply_vibrancy(&window, material, None, None) {
            crate::tynd_log!("window-vibrancy: {e}");
        }
    }

    #[cfg(target_os = "windows")]
    if let Some(ref v) = cfg.vibrancy {
        match v.as_str() {
            "mica" | "tabbed" => {
                if let Err(e) = window_vibrancy::apply_mica(&window, None) {
                    crate::tynd_log!("window-vibrancy mica: {e}");
                }
            },
            _ => {
                // "acrylic", "blur", or any unknown value → acrylic frosted glass.
                if let Err(e) =
                    window_vibrancy::apply_acrylic(&window, Some((18, 18, 18, 125)))
                {
                    crate::tynd_log!("window-vibrancy acrylic: {e}");
                }
            },
        }
    }

    window
}

/// Center a window on its current monitor.
pub fn center_window(window: &Window) {
    let Some(monitor) = window.current_monitor() else {
        return;
    };
    let screen = monitor.size();
    let win_size = window.outer_size();
    let screen_w = i32::try_from(screen.width).unwrap_or(i32::MAX);
    let screen_h = i32::try_from(screen.height).unwrap_or(i32::MAX);
    let win_w = i32::try_from(win_size.width).unwrap_or(i32::MAX);
    let win_h = i32::try_from(win_size.height).unwrap_or(i32::MAX);
    let x = (screen_w - win_w) / 2 + monitor.position().x;
    let y = (screen_h - win_h) / 2 + monitor.position().y;
    window.set_outer_position(tao::dpi::PhysicalPosition::new(x, y));
}

/// Fallback HTML shown when no `frontendDir` or `devUrl` is configured.
pub fn placeholder_html(app_name: &str) -> String {
    let escaped = app_name
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;");
    format!(
        r#"<!DOCTYPE html>
<html>
<head><meta charset="utf-8"><style>
  body {{ margin:0; background:#1a1a1a; color:#aaa; font-family:system-ui,sans-serif;
         display:flex; align-items:center; justify-content:center; height:100vh; flex-direction:column; }}
  h2   {{ color:#fff; margin:0 0 8px }}
  p    {{ margin:0; font-size:14px }}
</style></head>
<body>
  <h2>{escaped}</h2>
  <p>No frontend configured. Set <code>frontendDir</code> or <code>devUrl</code> in <code>app.start()</code>.</p>
</body>
</html>"#
    )
}
