use image::GenericImageView;

fn load_rgba(path: &str) -> Result<(Vec<u8>, u32, u32), String> {
    let img = image::open(path)
        .map_err(|e| format!("Cannot load icon '{path}': {e}"))?;
    let (w, h) = img.dimensions();
    Ok((img.into_rgba8().into_raw(), w, h))
}

/// Load an image and return a tao `Icon` (used for the window icon).
pub fn load_tao(path: &str) -> Result<tao::window::Icon, String> {
    let (rgba, w, h) = load_rgba(path)?;
    tao::window::Icon::from_rgba(rgba, w, h).map_err(|e| e.to_string())
}

/// Load an image and return a `tray_icon::Icon` (used for the system tray).
pub fn load_tray(path: &str) -> Result<tray_icon::Icon, String> {
    let (rgba, w, h) = load_rgba(path)?;
    tray_icon::Icon::from_rgba(rgba, w, h).map_err(|e| e.to_string())
}
