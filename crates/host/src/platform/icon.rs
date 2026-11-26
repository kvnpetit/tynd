//! Icon loading for windows and the tray: decode an image file, confined to
//! the filesystem allow-list, into raw RGBA.

use image::GenericImageView;

pub(crate) fn load_rgba(path: &str) -> Result<(Vec<u8>, u32, u32), String> {
    // Gate every icon path through the fs allow-list. Without this, frontend
    // JS calling `window.setIcon` / `tray.setIcon` could pass an arbitrary
    // path (`/etc/shadow`, UNC `\\attacker\share\..`) and the host would
    // happily try to decode it as an image, leaking the bytes via decode
    // errors or simply touching files outside the configured scope.
    // Use the resolved path returned by the gate. Checking one spelling and
    // then opening the caller spelling would leave a symlink escape intact.
    let resolved = crate::security::policy::authorize_fs_path(path)?;
    let img = image::open(&resolved).map_err(|e| format!("Cannot load icon '{path}': {e}"))?;
    let (w, h) = img.dimensions();
    Ok((img.into_rgba8().into_raw(), w, h))
}

/// Load an image and return a tao `Icon` (used for the window icon).
pub(crate) fn load_tao(path: &str) -> Result<tao::window::Icon, String> {
    let (rgba, w, h) = load_rgba(path)?;
    tao::window::Icon::from_rgba(rgba, w, h).map_err(|e| e.to_string())
}

/// Load an image and return a `tray_icon::Icon` (used for the system tray).
#[cfg(feature = "api-tray")]
pub(crate) fn load_tray(path: &str) -> Result<tray_icon::Icon, String> {
    let (rgba, w, h) = load_rgba(path)?;
    tray_icon::Icon::from_rgba(rgba, w, h).map_err(|e| e.to_string())
}
