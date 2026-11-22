//! GTK side of the frame controls. Untested on CI builds made from Windows.

use gtk::prelude::WidgetExt;
use tao::{
    platform::unix::WindowExtUnix,
    window::{Fullscreen, Window},
};

use super::TopLevel;

/// tao starts GTK resize drags on frameless windows.
pub(super) const SUPPORTS_EDGE_RESIZE: bool = true;

/// GTK has a single "keep above" layer: every level maps to it.
pub(super) fn set_top_level(_win: &Window, _level: TopLevel) {}

/// Needs a compositing window manager (X11) or compositor (Wayland).
pub(super) fn set_opacity(win: &Window, opacity: f64) -> Result<(), String> {
    win.gtk_window().set_opacity(opacity);
    Ok(())
}

pub(super) fn set_aspect_ratio(_win: &Window, _ratio: Option<f64>) -> Result<(), String> {
    Err("setAspectRatio: not supported on Linux".to_string())
}

pub(super) fn set_movable(_win: &Window, _movable: bool) -> Result<(), String> {
    Err("setMovable: not supported on Linux".to_string())
}

/// No separate "simple" mode: borderless fullscreen never switches workspaces.
pub(super) fn set_simple_fullscreen(win: &Window, fullscreen: bool) {
    win.set_fullscreen(fullscreen.then_some(Fullscreen::Borderless(None)));
}
