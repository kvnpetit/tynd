use tao::{
    dpi::{LogicalPosition, LogicalSize},
    event_loop::EventLoop,
    window::{Theme, Window, WindowBuilder},
};
pub(crate) use tynd_protocol::window::{
    GpuPreference, HardwareAcceleration, PreferredFps, TitleBarStyle, WindowConfig, WindowTheme,
};

/// Build the primary native window from `app.start({ window })`.
/// The window starts hidden — shown after the first page renders (no white
/// flash), or by `tyndWindow.show()` when `visible: false`.
pub(crate) fn build_window<T>(
    cfg: &WindowConfig,
    icon_path: Option<&str>,
    event_loop: &EventLoop<T>,
) -> Window {
    let mut wb = window_builder(cfg, (1200, 800));

    if let Some(path) = icon_path {
        match crate::platform::icon::load_tao(path) {
            Ok(icon) => {
                wb = wb.with_window_icon(Some(icon));
            },
            Err(e) => crate::tynd_warn!("Window icon '{}': {e}", path),
        }
    }

    let window = match wb.build(event_loop) {
        Ok(w) => w,
        Err(e) => crate::app::exit_startup_failure(&format!("Failed to create native window: {e}")),
    };
    apply_initial_state(&window, cfg);
    window
}

/// Native-window options shared by the primary window and secondary windows
/// (`multiwindow.create`). Always builds a hidden window: the caller shows
/// it once it is ready. `default_size` applies when `width` / `height` are unset.
pub(crate) fn window_builder(cfg: &WindowConfig, default_size: (u32, u32)) -> WindowBuilder {
    let mut wb = WindowBuilder::new()
        .with_title(cfg.title.as_deref().unwrap_or(""))
        .with_inner_size(LogicalSize::new(
            cfg.width.unwrap_or(default_size.0),
            cfg.height.unwrap_or(default_size.1),
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
    if let (Some(x), Some(y)) = (cfg.x, cfg.y) {
        wb = wb.with_position(LogicalPosition::new(x, y));
    }
    if let Some(theme) = cfg.theme {
        wb = wb.with_theme(tao_theme(theme));
    }
    if let Some(color) = cfg.background_color {
        wb = wb.with_background_color((color.r, color.g, color.b, color.a.unwrap_or(255)));
    }
    if cfg.skip_taskbar.unwrap_or(false) {
        wb = with_skip_taskbar(wb);
    }
    with_title_bar_style(wb, cfg.title_bar_style)
}

/// State applied after the window exists: maximized, fullscreen, kiosk, center.
pub(crate) fn apply_initial_state(window: &Window, cfg: &WindowConfig) {
    if cfg.maximized.unwrap_or(false) {
        window.set_maximized(true);
    }
    if cfg.fullscreen.unwrap_or(false) {
        window.set_fullscreen(Some(tao::window::Fullscreen::Borderless(None)));
    }
    if cfg.kiosk.unwrap_or(false) {
        crate::api::window_cmd::set_kiosk(window, true);
    }
    if cfg.center.unwrap_or(false) {
        center_window(window);
    }
}

/// Reject window options a renderer could send that `app.start()`'s schema
/// would have refused (zero sizes, non-finite positions).
pub(crate) fn validate_window_config(cfg: &WindowConfig) -> Result<(), String> {
    let sizes = [
        ("width", cfg.width),
        ("height", cfg.height),
        ("minWidth", cfg.min_width),
        ("minHeight", cfg.min_height),
        ("maxWidth", cfg.max_width),
        ("maxHeight", cfg.max_height),
    ];
    if let Some((name, _)) = sizes.iter().find(|(_, value)| *value == Some(0)) {
        return Err(format!("'{name}' must be a positive integer"));
    }
    for (name, value) in [("x", cfg.x), ("y", cfg.y)] {
        if value.is_some_and(|v| !v.is_finite()) {
            return Err(format!("'{name}' must be a finite number"));
        }
    }
    Ok(())
}

fn tao_theme(theme: WindowTheme) -> Option<Theme> {
    match theme {
        WindowTheme::Light => Some(Theme::Light),
        WindowTheme::Dark => Some(Theme::Dark),
        _ => None,
    }
}

#[cfg(target_os = "windows")]
fn with_skip_taskbar(wb: WindowBuilder) -> WindowBuilder {
    use tao::platform::windows::WindowBuilderExtWindows;
    wb.with_skip_taskbar(true)
}

#[cfg(all(unix, not(target_os = "macos")))]
fn with_skip_taskbar(wb: WindowBuilder) -> WindowBuilder {
    use tao::platform::unix::WindowBuilderExtUnix;
    wb.with_skip_taskbar(true)
}

/// macOS has no per-window taskbar entry (see `app.setDockVisible`).
#[cfg(target_os = "macos")]
fn with_skip_taskbar(wb: WindowBuilder) -> WindowBuilder {
    wb
}

/// Whether the primary window is shown automatically once its page is ready.
pub(crate) fn shows_on_page_ready(cfg: &WindowConfig) -> bool {
    cfg.visible.unwrap_or(true)
}

#[cfg(target_os = "macos")]
fn with_title_bar_style(wb: WindowBuilder, style: Option<TitleBarStyle>) -> WindowBuilder {
    use tao::platform::macos::WindowBuilderExtMacOS;
    match style {
        Some(TitleBarStyle::Transparent) => wb.with_titlebar_transparent(true),
        Some(TitleBarStyle::Overlay) => wb
            .with_titlebar_transparent(true)
            .with_title_hidden(true)
            .with_fullsize_content_view(true),
        _ => wb,
    }
}

/// Title bar styles are a macOS concept; Windows and Linux keep their
/// native title bar (use `decorations: false` plus `data-tynd-drag`).
#[cfg(not(target_os = "macos"))]
fn with_title_bar_style(wb: WindowBuilder, _style: Option<TitleBarStyle>) -> WindowBuilder {
    wb
}

/// Center a window on its current monitor.
pub(crate) fn center_window(window: &Window) {
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
    window.set_outer_position(dpi::PhysicalPosition::new(x, y));
}

/// Fallback HTML shown when no `frontendDir` or `devUrl` is configured.
pub(crate) fn placeholder_html(app_name: &str) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn config(json: &str) -> WindowConfig {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn validation_rejects_zero_sizes_and_accepts_defaults() {
        assert!(validate_window_config(&WindowConfig::default()).is_ok());
        assert!(validate_window_config(&config(r#"{"width":640,"x":-20,"y":0}"#)).is_ok());
        let error = validate_window_config(&config(r#"{"minHeight":0}"#)).unwrap_err();
        assert!(error.contains("minHeight"), "{error}");
    }

    #[test]
    fn system_theme_follows_the_os() {
        assert_eq!(tao_theme(WindowTheme::Dark), Some(Theme::Dark));
        assert_eq!(tao_theme(WindowTheme::Light), Some(Theme::Light));
        assert_eq!(tao_theme(WindowTheme::System), None);
    }
}
