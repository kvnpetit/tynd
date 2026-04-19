use arboard::{Clipboard, ImageData};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use image::{ImageBuffer, ImageFormat, Rgba};
use serde_json::{json, Value};

pub fn dispatch(method: &str, args: &Value) -> Result<Value, String> {
    match method {
        "readText" => read_text(),
        "writeText" => write_text(args),
        "readHtml" => read_html(),
        "writeHtml" => write_html(args),
        "readImage" => read_image(),
        "writeImage" => write_image(args),
        "clear" => clear(),
        _ => Err(format!("clipboard.{method}: unknown method")),
    }
}

fn new_cb() -> Result<Clipboard, String> {
    Clipboard::new().map_err(|e| format!("clipboard: {e}"))
}

fn read_text() -> Result<Value, String> {
    let mut cb = new_cb()?;
    let text = cb.get_text().unwrap_or_default();
    Ok(Value::String(text))
}

fn write_text(args: &Value) -> Result<Value, String> {
    let text = args
        .as_str()
        .or_else(|| args.get("text").and_then(|t| t.as_str()))
        .unwrap_or("");
    let mut cb = new_cb()?;
    cb.set_text(text)
        .map_err(|e| format!("clipboard.writeText: {e}"))?;
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
        .ok_or_else(|| "clipboard.writeHtml: missing 'html'".to_string())?;
    // Plain-text fallback for apps that don't understand text/html.
    let alt = args.get("alt").and_then(Value::as_str);
    let mut cb = new_cb()?;
    cb.set_html(html, alt)
        .map_err(|e| format!("clipboard.writeHtml: {e}"))?;
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
        .ok_or_else(|| "clipboard.readImage: bad RGBA buffer".to_string())?;
    let mut png = Vec::with_capacity((w as usize) * (h as usize));
    let mut cursor = std::io::Cursor::new(&mut png);
    buffer
        .write_to(&mut cursor, ImageFormat::Png)
        .map_err(|e| format!("clipboard.readImage: encode: {e}"))?;

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
        .ok_or_else(|| "clipboard.writeImage: missing 'png'".to_string())?;
    let png_bytes = STANDARD
        .decode(png_b64)
        .map_err(|e| format!("clipboard.writeImage: base64: {e}"))?;
    let dynimg = image::load_from_memory_with_format(&png_bytes, ImageFormat::Png)
        .map_err(|e| format!("clipboard.writeImage: decode: {e}"))?;
    let rgba = dynimg.to_rgba8();
    let (w, h) = rgba.dimensions();

    let data = ImageData {
        width: w as usize,
        height: h as usize,
        bytes: rgba.into_raw().into(),
    };
    let mut cb = new_cb()?;
    cb.set_image(data)
        .map_err(|e| format!("clipboard.writeImage: {e}"))?;
    Ok(Value::Null)
}

fn clear() -> Result<Value, String> {
    let mut cb = new_cb()?;
    cb.clear().map_err(|e| format!("clipboard.clear: {e}"))?;
    Ok(Value::Null)
}
