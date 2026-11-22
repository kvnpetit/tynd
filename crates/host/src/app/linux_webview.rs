use gtk::prelude::*;
use tao::{dpi, platform::unix::WindowExtUnix, window::Window};
use wry::{WebView, WebViewBuilder, WebViewBuilderExtUnix};

/// GTK composition surface shared by the root WebView and every child
/// WebView in one native window.
///
/// The root WebView is the main child of `GtkOverlay`, so GTK sizes it to the
/// available content area (including menu-bar subtraction). Child WebViews
/// live in the overlaid `GtkFixed`, which gives Wry mutable bounds on both X11
/// and Wayland.
pub(super) struct LinuxWebviewContainer {
    _overlay: gtk::Overlay,
    children: gtk::Fixed,
}

impl LinuxWebviewContainer {
    pub(super) fn new(window: &Window) -> Result<Self, String> {
        let vbox = window
            .default_vbox()
            .ok_or_else(|| "tao window has no default GTK content box".to_string())?;

        let overlay = gtk::Overlay::new();
        overlay.set_hexpand(true);
        overlay.set_vexpand(true);

        let children = gtk::Fixed::new();
        children.set_hexpand(true);
        children.set_vexpand(true);
        overlay.add_overlay(&children);
        vbox.pack_start(&overlay, true, true, 0);

        children.show();
        overlay.show();

        Ok(Self {
            _overlay: overlay,
            children,
        })
    }

    pub(super) fn build_root<'a>(&'a self, builder: WebViewBuilder<'a>) -> wry::Result<WebView> {
        builder.build_gtk(&self._overlay)
    }

    pub(super) fn build_child<'a>(&'a self, builder: WebViewBuilder<'a>) -> wry::Result<WebView> {
        builder.build_gtk(&self.children)
    }

    /// Logical content size for `autoResize`. Before GTK's first allocation,
    /// fall back to Tao's window size so hidden-at-start windows still get a
    /// useful initial child size.
    pub(super) fn logical_size(&self, window: &Window) -> dpi::LogicalSize<f64> {
        let width = self.children.allocated_width();
        let height = self.children.allocated_height();
        if width > 1 && height > 1 {
            dpi::LogicalSize::new(f64::from(width), f64::from(height))
        } else {
            window.inner_size().to_logical::<f64>(window.scale_factor())
        }
    }
}
