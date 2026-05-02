//! Screen + window capture.
//!
//! Backed by `xcap`, which wraps the platform-native capture APIs:
//! - macOS: `CGDisplayCreateImage` / `CGWindowListCreateImage`
//! - Windows: DXGI desktop duplication / `BitBlt` fallback
//! - Linux: X11 `XGetImage` or PipeWire (Wayland)
//!
//! Captures arrive as `RgbaImage`; we encode to PNG before returning so
//! the wire payload is browser-displayable as `data:image/png;base64,…`
//! without an extra round-trip through `image` on the JS side.

use base64::{engine::general_purpose::STANDARD, Engine as _};
use image::ImageFormat;
use serde_json::{json, Value};
use xcap::{Monitor, Window};

pub fn dispatch(method: &str, args: &Value) -> Result<Value, String> {
    // W3.2 — ACL gate. The two `capture*` methods are the privacy-
    // sensitive surface (anything that returns a frame buffer of the
    // user's screen). The `list*` introspection methods are kept open
    // so admin / settings UIs can populate a monitor / window picker
    // *before* the user grants `screenshot:captureScreen` — same
    // posture as `biometric.isAvailable`.
    #[cfg(feature = "api-acl")]
    {
        let needs_gate = matches!(method, "captureScreen" | "captureWindow");
        if needs_gate {
            let cmd = format!("screenshot:{method}");
            if !crate::os::acl::authority().is_allowed(&cmd, None, None, None) {
                return Err(format!("screenshot.{method}: permission denied by ACL"));
            }
        }
    }
    match method {
        "captureScreen" => capture_screen(args),
        "captureWindow" => capture_window(args),
        "listMonitors" => list_monitors(),
        "listWindows" => list_windows(),
        _ => Err(format!("screenshot.{method}: unknown method")),
    }
}

fn list_monitors() -> Result<Value, String> {
    let monitors = Monitor::all().map_err(|e| format!("screenshot.listMonitors: {e}"))?;
    let arr: Vec<Value> = monitors
        .iter()
        .map(|m| {
            json!({
                "id": m.id(),
                "name": m.name(),
                "width": m.width(),
                "height": m.height(),
                "scale": m.scale_factor(),
                "primary": m.is_primary(),
            })
        })
        .collect();
    Ok(Value::Array(arr))
}

fn list_windows() -> Result<Value, String> {
    let windows = Window::all().map_err(|e| format!("screenshot.listWindows: {e}"))?;
    let arr: Vec<Value> = windows
        .iter()
        .map(|w| {
            json!({
                "id": w.id(),
                "title": w.title(),
                "app_name": w.app_name(),
                "width": w.width(),
                "height": w.height(),
            })
        })
        .collect();
    Ok(Value::Array(arr))
}

fn capture_screen(args: &Value) -> Result<Value, String> {
    let monitors = Monitor::all().map_err(|e| format!("screenshot.captureScreen: {e}"))?;
    if monitors.is_empty() {
        return Err("screenshot.captureScreen: no monitors detected".into());
    }
    let idx = args
        .get("monitor_index")
        .or_else(|| args.get("monitorIndex"))
        .and_then(Value::as_u64)
        .unwrap_or(0) as usize;
    let monitor = monitors
        .get(idx)
        .ok_or_else(|| format!("screenshot.captureScreen: monitor {idx} out of range"))?;

    let img = monitor
        .capture_image()
        .map_err(|e| format!("screenshot.captureScreen: {e}"))?;
    encode_png(img.width(), img.height(), &img.into_raw())
}

fn capture_window(args: &Value) -> Result<Value, String> {
    let id = args
        .get("window_id")
        .or_else(|| args.get("windowId"))
        .and_then(Value::as_u64)
        .ok_or_else(|| "screenshot.captureWindow: missing 'window_id'".to_string())?;

    let windows = Window::all().map_err(|e| format!("screenshot.captureWindow: {e}"))?;
    let id_u32: u32 = u32::try_from(id)
        .map_err(|_| format!("screenshot.captureWindow: window_id {id} out of u32 range"))?;
    let window = windows
        .into_iter()
        .find(|w| w.id() == id_u32)
        .ok_or_else(|| format!("screenshot.captureWindow: window {id} not found"))?;

    let img = window
        .capture_image()
        .map_err(|e| format!("screenshot.captureWindow: {e}"))?;
    encode_png(img.width(), img.height(), &img.into_raw())
}

/// Encode RGBA bytes as PNG and wrap in the standard `{ width, height,
/// data }` shape. `data` is base64 to keep the JSON wire contract intact —
/// a future zero-copy path can route through `tynd-bin://` (see
/// `scheme_bin.rs`) the same way `fs.readBinary` does, but PNG-encoded
/// screenshots are typically <2 MB so the base64 cost is acceptable.
fn encode_png(width: u32, height: u32, rgba: &[u8]) -> Result<Value, String> {
    if width == 0 || height == 0 {
        return Err(format!(
            "screenshot: invalid capture dimensions ({width}x{height})"
        ));
    }
    let buffer = image::RgbaImage::from_raw(width, height, rgba.to_vec())
        .ok_or_else(|| "screenshot: malformed RGBA buffer".to_string())?;
    let mut png = Vec::with_capacity((width as usize) * (height as usize));
    let mut cursor = std::io::Cursor::new(&mut png);
    buffer
        .write_to(&mut cursor, ImageFormat::Png)
        .map_err(|e| format!("screenshot: PNG encode: {e}"))?;
    Ok(json!({
        "width": width,
        "height": height,
        "data": STANDARD.encode(&png),
    }))
}
