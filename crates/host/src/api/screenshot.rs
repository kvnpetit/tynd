//! Screen + window capture via `xcap` (CGDisplay / DXGI / X11 / PipeWire).
//!
//! Captures encode to PNG before returning so the wire payload renders as
//! `data:image/png;base64,…` without a JS-side `image` round-trip.
//!
//! `captureScreen` / `captureWindow` are ACL-gated.

use base64::{engine::general_purpose::STANDARD, Engine as _};
use image::ImageFormat;
use serde_json::{json, Value};
use xcap::{Monitor, Window};

use crate::api::HostApi;
use crate::ipc::error::IpcError;

// Crate-visible because handler macro emits public dispatch functions.
// Public methods are macro inputs; keeping the module crate-private prevents ACL bypass.
#[allow(unreachable_pub)]
pub(crate) mod inner {
    use serde_json::Value;

    use super::{capture_screen_impl, capture_window_impl, list_monitors_impl, list_windows_impl};
    use crate::tynd;

    /// Stateless dispatch marker.
    #[derive(Debug, Default)]
    pub(crate) struct ScreenshotApi;

    #[tynd::handler(api = "screenshot")]
    impl ScreenshotApi {
        /// Capture the framebuffer of the monitor at `monitorIndex`
        /// (default `0`, i.e. the primary display). Returns
        /// `{ width, height, data }` where `data` is a base64-encoded
        /// PNG.
        pub fn capture_screen(monitor_index: Option<u64>) -> Result<Value, String> {
            capture_screen_impl(monitor_index.unwrap_or(0) as usize)
        }

        /// Capture a single window by `windowId` (as returned by
        /// `listWindows`). Returns the same `{ width, height, data }`
        /// PNG shape as `captureScreen`.
        pub fn capture_window(window_id: u64) -> Result<Value, String> {
            capture_window_impl(window_id)
        }

        /// List every monitor known to the OS, with `id`, `name`, pixel
        /// dimensions, scale factor, and a `primary` flag.
        pub fn list_monitors() -> Result<Value, String> {
            list_monitors_impl()
        }

        /// List every top-level window with `id`, `title`, `app_name`,
        /// and pixel dimensions.
        pub fn list_windows() -> Result<Value, String> {
            list_windows_impl()
        }
    }
}

/// Row of the host API table (`api::HOST_APIS`).
pub(crate) const API: HostApi = HostApi::new("screenshot", |method, args| dispatch(method, &args));

/// Public dispatch wrapper — interposes the ACL gate on the two
/// privacy-sensitive `capture*` methods before delegating to the
/// macro-generated table inside [`inner`].
pub(crate) fn dispatch(method: &str, args: &Value) -> Result<Value, String> {
    // ACL gate on the privacy-sensitive `capture*` methods. `list*` is
    // ungated so settings UIs can build a picker before the user grants.
    let needs_gate = matches!(method, "captureScreen" | "captureWindow");
    if needs_gate {
        let cmd = format!("screenshot:{method}");
        if !crate::security::acl::authority().is_allowed(&cmd, None, None, None) {
            return Err(shot_err(method, "permission denied by ACL"));
        }
    }
    inner::dispatch(method, args)
}

/// Build the `screenshot.<method>: <msg>` IPC error string.
#[cold]
fn shot_err(method: &str, msg: impl std::fmt::Display) -> String {
    IpcError::new("screenshot", method, msg).into_string()
}

fn list_monitors_impl() -> Result<Value, String> {
    let monitors = Monitor::all().map_err(|e| shot_err("listMonitors", e))?;
    let arr = monitors
        .iter()
        .map(|m| -> xcap::XCapResult<Value> {
            Ok(json!({
                "id": m.id()?,
                "name": m.name()?,
                "width": m.width()?,
                "height": m.height()?,
                "scale": m.scale_factor()?,
                "primary": m.is_primary()?,
            }))
        })
        .collect::<Result<Vec<Value>, _>>()
        .map_err(|e| shot_err("listMonitors", e))?;
    Ok(Value::Array(arr))
}

fn list_windows_impl() -> Result<Value, String> {
    let windows = Window::all().map_err(|e| shot_err("listWindows", e))?;
    // Windows can disappear between enumeration and property reads; skip
    // the ones whose id is gone and blank the other unreadable fields.
    let arr: Vec<Value> = windows
        .iter()
        .filter_map(|w| {
            Some(json!({
                "id": w.id().ok()?,
                "title": w.title().unwrap_or_default(),
                "app_name": w.app_name().unwrap_or_default(),
                "width": w.width().unwrap_or_default(),
                "height": w.height().unwrap_or_default(),
            }))
        })
        .collect();
    Ok(Value::Array(arr))
}

fn capture_screen_impl(idx: usize) -> Result<Value, String> {
    let monitors = Monitor::all().map_err(|e| shot_err("captureScreen", e))?;
    if monitors.is_empty() {
        return Err(shot_err("captureScreen", "no monitors detected"));
    }
    let monitor = monitors
        .get(idx)
        .ok_or_else(|| shot_err("captureScreen", format!("monitor {idx} out of range")))?;

    let img = monitor
        .capture_image()
        .map_err(|e| shot_err("captureScreen", e))?;
    encode_png(img.width(), img.height(), &img.into_raw())
}

fn capture_window_impl(id: u64) -> Result<Value, String> {
    let windows = Window::all().map_err(|e| shot_err("captureWindow", e))?;
    let id_u32: u32 = u32::try_from(id)
        .map_err(|_| shot_err("captureWindow", format!("window_id {id} out of u32 range")))?;
    let window = windows
        .into_iter()
        .find(|w| w.id().is_ok_and(|wid| wid == id_u32))
        .ok_or_else(|| shot_err("captureWindow", format!("window {id} not found")))?;

    let img = window
        .capture_image()
        .map_err(|e| shot_err("captureWindow", e))?;
    encode_png(img.width(), img.height(), &img.into_raw())
}

/// Encode RGBA bytes as PNG and wrap in the standard `{ width, height,
/// data }` shape. `data` is base64 to keep the JSON wire contract intact —
/// a future zero-copy path can route through `tynd-bin://` (see
/// `scheme_bin.rs`) the same way `fs.readBinary` does, but PNG-encoded
/// screenshots are typically <2 MB so the base64 cost is acceptable.
fn encode_png(width: u32, height: u32, rgba: &[u8]) -> Result<Value, String> {
    if width == 0 || height == 0 {
        return Err(shot_err(
            "encode_png",
            format!("invalid capture dimensions ({width}x{height})"),
        ));
    }
    let buffer = image::RgbaImage::from_raw(width, height, rgba.to_vec())
        .ok_or_else(|| shot_err("encode_png", "malformed RGBA buffer"))?;
    let mut png = Vec::with_capacity((width as usize) * (height as usize));
    let mut cursor = std::io::Cursor::new(&mut png);
    buffer
        .write_to(&mut cursor, ImageFormat::Png)
        .map_err(|e| shot_err("encode_png", e))?;
    Ok(json!({
        "width": width,
        "height": height,
        "data": STANDARD.encode(&png),
    }))
}
