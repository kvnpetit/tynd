use std::sync::{
    atomic::{AtomicBool, Ordering},
    LazyLock,
};

use arboard::{Clipboard, ImageData};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use image::{ImageBuffer, ImageFormat, Rgba};
use parking_lot::Mutex;
use serde_json::{json, Value};

use super::events;
use crate::ipc_error::IpcError;

pub fn dispatch(method: &str, args: &Value) -> Result<Value, String> {
    match method {
        "readText" => read_text(),
        "writeText" => write_text(args),
        "readHtml" => read_html(),
        "writeHtml" => write_html(args),
        "readImage" => read_image(),
        "writeImage" => write_image(args),
        "clear" => clear(),
        "startMonitoring" => start_monitoring(args),
        "stopMonitoring" => stop_monitoring(),
        _ => Err(IpcError::unknown_method("clipboard", method).into_string()),
    }
}

fn new_cb() -> Result<Clipboard, String> {
    Clipboard::new().map_err(|e| IpcError::new("clipboard", "*", e).into_string())
}

fn read_text() -> Result<Value, String> {
    let mut cb = new_cb()?;
    // arboard returns `Err(ContentNotAvailable)` for empty clipboard — that's
    // a legitimate empty case, not an error. Other errors (X11 lock, perm)
    // must propagate so callers can distinguish "empty" from "broken".
    match cb.get_text() {
        Ok(t) => Ok(Value::String(t)),
        Err(arboard::Error::ContentNotAvailable) => Ok(Value::String(String::new())),
        Err(e) => Err(IpcError::new("clipboard", "readText", e).into_string()),
    }
}

fn write_text(args: &Value) -> Result<Value, String> {
    let text = args
        .as_str()
        .or_else(|| args.get("text").and_then(|t| t.as_str()))
        .unwrap_or("");
    let mut cb = new_cb()?;
    cb.set_text(text)
        .map_err(|e| IpcError::new("clipboard", "writeText", e).into_string())?;
    Ok(Value::Null)
}

#[allow(clippy::unnecessary_wraps)]
fn read_html() -> Result<Value, String> {
    // arboard doesn't expose a read_html — the clipboard only holds one
    // payload at a time, so a previous `writeHtml` is observable via
    // `readText` (browsers flatten HTML to text). Report null to signal
    // "not supported" rather than pretend.
    Ok(Value::Null)
}

fn write_html(args: &Value) -> Result<Value, String> {
    let html = args
        .get("html")
        .and_then(Value::as_str)
        .ok_or_else(|| String::from(IpcError::missing_arg("clipboard", "writeHtml", "html")))?;
    // Plain-text fallback for apps that don't understand text/html.
    let alt = args.get("alt").and_then(Value::as_str);
    let mut cb = new_cb()?;
    cb.set_html(html, alt)
        .map_err(|e| IpcError::new("clipboard", "writeHtml", e).into_string())?;
    Ok(Value::Null)
}

/// Read the clipboard image as a base64 PNG. Returns null if empty or
/// non-image. Encoding RGBA -> PNG keeps the wire format identical to
/// `writeImage` and `fs.readBinary`.
fn read_image() -> Result<Value, String> {
    let mut cb = new_cb()?;
    let Ok(img) = cb.get_image() else {
        return Ok(Value::Null);
    };

    let (w, h) = (
        u32::try_from(img.width).unwrap_or(0),
        u32::try_from(img.height).unwrap_or(0),
    );
    if w == 0 || h == 0 {
        return Ok(Value::Null);
    }
    let buffer: ImageBuffer<Rgba<u8>, _> = ImageBuffer::from_raw(w, h, img.bytes.into_owned())
        .ok_or_else(|| {
            IpcError::new("clipboard", "readImage", "bad RGBA buffer").into_string()
        })?;
    let mut png = Vec::with_capacity((w as usize) * (h as usize));
    let mut cursor = std::io::Cursor::new(&mut png);
    buffer
        .write_to(&mut cursor, ImageFormat::Png)
        .map_err(|e| IpcError::new("clipboard", "readImage", format!("encode: {e}")).into_string())?;

    Ok(json!({
        "png": STANDARD.encode(&png),
        "width": w,
        "height": h,
    }))
}

/// Accept a base64 PNG, decode to RGBA, hand to arboard. Keeps the TS API
/// symmetric with `readImage` and with `fs.writeBinary`.
fn write_image(args: &Value) -> Result<Value, String> {
    let png_b64 = args
        .get("png")
        .and_then(Value::as_str)
        .ok_or_else(|| String::from(IpcError::missing_arg("clipboard", "writeImage", "png")))?;
    let png_bytes = STANDARD.decode(png_b64).map_err(|e| {
        IpcError::new("clipboard", "writeImage", format!("base64: {e}")).into_string()
    })?;
    let dynimg = image::load_from_memory_with_format(&png_bytes, ImageFormat::Png).map_err(
        |e| IpcError::new("clipboard", "writeImage", format!("decode: {e}")).into_string(),
    )?;
    let rgba = dynimg.to_rgba8();
    let (w, h) = rgba.dimensions();

    let data = ImageData {
        width: w as usize,
        height: h as usize,
        bytes: rgba.into_raw().into(),
    };
    let mut cb = new_cb()?;
    cb.set_image(data)
        .map_err(|e| IpcError::new("clipboard", "writeImage", e).into_string())?;
    Ok(Value::Null)
}

fn clear() -> Result<Value, String> {
    let mut cb = new_cb()?;
    cb.clear()
        .map_err(|e| IpcError::new("clipboard", "clear", e).into_string())?;
    Ok(Value::Null)
}

// No cross-OS event API for the clipboard — poll on a background thread.
// 200ms is fast enough for typical paste flows and cheap (single text read
// + hash compare). Image changes are detected too, via the same text path:
// arboard returns an error when the clipboard holds a non-text payload, so
// the "last seen" marker just notes transitions.

static MONITORING: AtomicBool = AtomicBool::new(false);
/// `LazyLock` initializes the inner `Mutex<u64>` to 0 on first access
/// — replaces a `last_hash()` accessor wrapping `OnceLock::get_or_init`.
static LAST_HASH: LazyLock<Mutex<u64>> = LazyLock::new(|| Mutex::new(0));
const DEFAULT_POLL_MS: u64 = 200;

#[allow(clippy::unnecessary_wraps)]
fn start_monitoring(args: &Value) -> Result<Value, String> {
    // AcqRel pairs with the Acquire load in the poll loop + Release store
    // in `stop_monitoring`.
    if MONITORING.swap(true, Ordering::AcqRel) {
        return Ok(Value::Null);
    }
    let interval = args
        .get("intervalMs")
        .and_then(Value::as_u64)
        .unwrap_or(DEFAULT_POLL_MS)
        .max(50);

    // Seed with the current content so the first tick doesn't fire
    // spuriously. The poll loop uses `get_text().unwrap_or_default()`, so
    // a failed seed MUST degrade to the hash of an empty string (not 0) —
    // otherwise the first successful poll always reports a change.
    let initial = Clipboard::new()
        .and_then(|mut c| c.get_text())
        .unwrap_or_default();
    *LAST_HASH.lock() = hash(initial.as_bytes());

    std::thread::spawn(move || {
        while MONITORING.load(Ordering::Acquire) {
            std::thread::sleep(std::time::Duration::from_millis(interval));
            let Ok(mut cb) = Clipboard::new() else {
                continue;
            };
            let text = cb.get_text().unwrap_or_default();
            let h = hash(text.as_bytes());
            let mut last = LAST_HASH.lock();
            if h != *last {
                *last = h;
                drop(last);
                events::emit("clipboard:change", &json!({ "text": text }));
            }
        }
    });
    Ok(Value::Null)
}

#[allow(clippy::unnecessary_wraps)]
fn stop_monitoring() -> Result<Value, String> {
    MONITORING.store(false, Ordering::Release);
    Ok(Value::Null)
}

fn hash(bytes: &[u8]) -> u64 {
    // ahash = non-crypto, collision-resistant enough for change detection.
    use std::hash::{BuildHasher as _, Hasher as _};
    let mut h = ahash::RandomState::with_seeds(1, 2, 3, 4).build_hasher();
    h.write(bytes);
    h.finish()
}
