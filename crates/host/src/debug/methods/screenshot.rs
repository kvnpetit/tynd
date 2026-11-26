//! `screenshot.*` — capture a Tynd window.

use serde_json::Value;
use tynd_debug_protocol::protocol::RpcError;

use super::CallOutcome;

// Cargo features cannot be target-specific, so `debug-screenshot` (on in the
// runtime preset) links `xcap` on Windows too, where the debug capture goes
// through GDI. A renamed non-Windows-only copy of the dependency is rejected by
// Cargo (same package under two names), so mark the crate as intentionally
// unused here instead of letting every Windows build warn.
#[cfg(all(
    target_os = "windows",
    feature = "debug-screenshot",
    not(feature = "api-screenshot")
))]
use xcap as _;

#[cfg(any(
    feature = "api-screenshot",
    feature = "debug-screenshot",
    target_os = "windows"
))]
fn capture(params: &Value, max_dimension: Option<u32>) -> CallOutcome {
    use base64::Engine as _;
    use image::ImageFormat;
    let mut img = match capture_platform(params) {
        Ok(image) => image,
        Err(error) => return CallOutcome::Error(error),
    };
    if let Some(max) = max_dimension {
        if img.width() > max || img.height() > max {
            img = image::imageops::thumbnail(&img, max, max);
        }
    }
    let mut buf: Vec<u8> = Vec::with_capacity(img.len());
    if let Err(e) = img.write_to(&mut std::io::Cursor::new(&mut buf), ImageFormat::Png) {
        return CallOutcome::Error(RpcError::internal_error(format!("png encode: {e}")));
    }
    let b64 = base64::engine::general_purpose::STANDARD.encode(&buf);
    CallOutcome::Result(serde_json::json!({
        "width": img.width(),
        "height": img.height(),
        "png_base64": b64,
    }))
}

#[cfg(all(
    any(
        feature = "api-screenshot",
        feature = "debug-screenshot",
        target_os = "windows"
    ),
    target_os = "windows"
))]
fn capture_platform(params: &Value) -> Result<image::RgbaImage, RpcError> {
    windows_capture::capture(params)
}

#[cfg(all(
    any(
        feature = "api-screenshot",
        feature = "debug-screenshot",
        target_os = "windows"
    ),
    target_os = "windows"
))]
mod windows_capture {
    use windows_sys::Win32::Foundation::HWND;

    use super::{RpcError, Value};

    pub(super) fn capture(params: &Value) -> Result<image::RgbaImage, RpcError> {
        let hwnd = window_handle(params)?;
        let (width, height) = capture_bounds(hwnd)?;
        let mut pixels = capture_pixels(hwnd, width, height)?;
        for pixel in pixels.as_chunks_mut::<4>().0 {
            pixel.swap(0, 2);
            pixel[3] = 255;
        }
        image::RgbaImage::from_raw(width as u32, height as u32, pixels)
            .ok_or_else(|| RpcError::internal_error("captured pixel buffer has invalid dimensions"))
    }

    fn window_handle(params: &Value) -> Result<HWND, RpcError> {
        let label = params
            .get("label")
            .and_then(Value::as_str)
            .unwrap_or("main");
        let info = super::super::window::query("info", &serde_json::json!({ "label": label }))?;
        let native_id = info
            .get("native_id")
            .and_then(Value::as_u64)
            .ok_or_else(|| RpcError::internal_error("window native handle unavailable"))?;
        Ok(native_id as usize as HWND)
    }

    fn capture_bounds(hwnd: HWND) -> Result<(i32, i32), RpcError> {
        use windows_sys::Win32::{Foundation::RECT, UI::WindowsAndMessaging::GetWindowRect};

        let mut rect = RECT::default();
        if unsafe { GetWindowRect(hwnd, &raw mut rect) } == 0 {
            return Err(RpcError::internal_error(format!(
                "GetWindowRect: {}",
                std::io::Error::last_os_error()
            )));
        }
        let width = rect.right.saturating_sub(rect.left);
        let height = rect.bottom.saturating_sub(rect.top);
        if width <= 0 || height <= 0 {
            return Err(RpcError::internal_error("window has empty capture bounds"));
        }
        Ok((width, height))
    }

    fn capture_pixels(hwnd: HWND, width: i32, height: i32) -> Result<Vec<u8>, RpcError> {
        use std::mem::size_of;
        use windows_sys::Win32::{
            Graphics::Gdi::{
                BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject,
                GetDIBits, GetWindowDC, ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER,
                BI_RGB, DIB_RGB_COLORS, SRCCOPY,
            },
            Storage::Xps::PrintWindow,
            UI::WindowsAndMessaging::PW_RENDERFULLCONTENT,
        };

        let window_dc = unsafe { GetWindowDC(hwnd) };
        if window_dc.is_null() {
            return Err(RpcError::internal_error("GetWindowDC returned null"));
        }
        let memory_dc = unsafe { CreateCompatibleDC(window_dc) };
        if memory_dc.is_null() {
            unsafe {
                ReleaseDC(hwnd, window_dc);
            }
            return Err(RpcError::internal_error("CreateCompatibleDC returned null"));
        }
        let bitmap = unsafe { CreateCompatibleBitmap(window_dc, width, height) };
        if bitmap.is_null() {
            unsafe {
                DeleteDC(memory_dc);
                ReleaseDC(hwnd, window_dc);
            }
            return Err(RpcError::internal_error(
                "CreateCompatibleBitmap returned null",
            ));
        }
        let old = unsafe { SelectObject(memory_dc, bitmap) };
        let printed = unsafe { PrintWindow(hwnd, memory_dc, PW_RENDERFULLCONTENT) } != 0;
        let rendered = printed
            || unsafe { BitBlt(memory_dc, 0, 0, width, height, window_dc, 0, 0, SRCCOPY) } != 0;
        unsafe {
            SelectObject(memory_dc, old);
        }

        let mut bitmap_info = BITMAPINFO::default();
        bitmap_info.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
        bitmap_info.bmiHeader.biWidth = width;
        bitmap_info.bmiHeader.biHeight = -height;
        bitmap_info.bmiHeader.biPlanes = 1;
        bitmap_info.bmiHeader.biBitCount = 32;
        bitmap_info.bmiHeader.biCompression = BI_RGB;
        let byte_len = (width as usize)
            .checked_mul(height as usize)
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or_else(|| RpcError::internal_error("capture dimensions overflow"))?;
        let mut pixels = vec![0_u8; byte_len];
        let lines = if rendered {
            unsafe {
                GetDIBits(
                    window_dc,
                    bitmap,
                    0,
                    height as u32,
                    pixels.as_mut_ptr().cast(),
                    &raw mut bitmap_info,
                    DIB_RGB_COLORS,
                )
            }
        } else {
            0
        };
        unsafe {
            DeleteObject(bitmap);
            DeleteDC(memory_dc);
            ReleaseDC(hwnd, window_dc);
        }
        if lines == 0 {
            return Err(RpcError::internal_error(format!(
                "window capture failed: {}",
                std::io::Error::last_os_error()
            )));
        }
        Ok(pixels)
    }
}
#[cfg(all(
    any(
        feature = "api-screenshot",
        feature = "debug-screenshot",
        target_os = "windows"
    ),
    not(target_os = "windows")
))]
fn capture_platform(params: &Value) -> Result<image::RgbaImage, RpcError> {
    use xcap::Window;

    let label = params
        .get("label")
        .and_then(Value::as_str)
        .unwrap_or("main");
    let title_filter = super::window::query("info", &serde_json::json!({ "label": label }))
        .ok()
        .and_then(|info| info.get("title").and_then(Value::as_str).map(str::to_owned))
        .or_else(|| {
            params
                .get("title")
                .and_then(Value::as_str)
                .map(str::to_owned)
        });
    let id_filter = params.get("id").and_then(Value::as_u64);
    let windows =
        Window::all().map_err(|error| RpcError::internal_error(format!("xcap: {error}")))?;
    let target = windows.iter().find(|window| {
        let id_match = id_filter.is_none_or(|id| window.id().is_ok_and(|wid| u64::from(wid) == id));
        let title_match = title_filter
            .as_deref()
            .is_none_or(|title| window.title().is_ok_and(|t| t == title));
        id_match && title_match
    });
    let Some(window) = target else {
        return Err(RpcError::invalid_params("no matching window"));
    };
    let captured = window
        .capture_image()
        .map_err(|error| RpcError::internal_error(format!("capture: {error}")))?;
    let (width, height) = (captured.width(), captured.height());
    image::RgbaImage::from_raw(width, height, captured.into_raw())
        .ok_or_else(|| RpcError::internal_error("captured pixel buffer has invalid dimensions"))
}

#[cfg(any(
    feature = "api-screenshot",
    feature = "debug-screenshot",
    target_os = "windows"
))]
pub(crate) fn window(params: Value) -> CallOutcome {
    capture(&params, None)
}

#[cfg(any(
    feature = "api-screenshot",
    feature = "debug-screenshot",
    target_os = "windows"
))]
pub(crate) fn thumbnail(params: Value) -> CallOutcome {
    capture(&params, Some(320))
}

#[cfg(not(any(
    feature = "api-screenshot",
    feature = "debug-screenshot",
    target_os = "windows"
)))]
pub(crate) fn window(_params: Value) -> CallOutcome {
    CallOutcome::Error(RpcError::method_not_found(
        "screenshot.window (host built without api-screenshot)",
    ))
}

#[cfg(not(any(
    feature = "api-screenshot",
    feature = "debug-screenshot",
    target_os = "windows"
)))]
pub(crate) fn thumbnail(_params: Value) -> CallOutcome {
    CallOutcome::Error(RpcError::method_not_found(
        "screenshot.thumbnail (host built without screenshot support)",
    ))
}
