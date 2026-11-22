//! Clipboard read/write + change monitoring.
//!
//! Backed by `arboard` for the cross-OS read/write surface and a
//! polling loop for change detection (no native subscribe API).

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, LazyLock,
};
use std::thread::JoinHandle;

use arboard::{Clipboard, ImageData};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use image::{ImageBuffer, ImageFormat, Rgba};
use parking_lot::Mutex;
use serde_json::{json, Value};

use crate::ipc::error::IpcError;

mod formats;

use super::watch_refs::{WatchRefs, BACKEND_OWNER};
use crate::api::HostApi;
use crate::ipc::events;
use crate::tynd;

/// Dispatch marker — poll-loop state lives in the file-level statics.
#[derive(Debug, Default)]
pub(crate) struct ClipboardApi;

/// Row of the host API table (`api::HOST_APIS`).
pub(crate) const API: HostApi = HostApi {
    dispatch_for_webview: Some(dispatch_for_webview),
    shutdown: Some(shutdown),
    release_webview: Some(release_webview),
    ..HostApi::new("clipboard", |method, args| dispatch(method, &args))
};

#[tynd::handler(api = "clipboard")]
impl ClipboardApi {
    /// Read clipboard text. Returns the empty string when the
    /// clipboard holds no text payload (not an error — empty is a
    /// legitimate state).
    pub fn read_text() -> Result<String, String> {
        let mut cb = new_cb()?;
        match cb.get_text() {
            Ok(t) => Ok(t),
            Err(arboard::Error::ContentNotAvailable) => Ok(String::new()),
            Err(e) => Err(IpcError::new("clipboard", "readText", e).into_string()),
        }
    }

    /// Write text to the clipboard.
    pub fn write_text(text: String) -> Result<(), String> {
        let mut cb = new_cb()?;
        cb.set_text(&text)
            .map_err(|e| IpcError::new("clipboard", "writeText", e).into_string())
    }

    /// Read clipboard HTML. Returns `null` when the clipboard has no
    /// HTML representation.
    pub fn read_html() -> Result<Value, String> {
        let mut cb = new_cb()?;
        optional_html(cb.get().html())
    }

    /// Write HTML to the clipboard. `alt` is the plain-text fallback
    /// used by apps that don't understand `text/html`.
    pub fn write_html(html: String, alt: Option<String>) -> Result<(), String> {
        let mut cb = new_cb()?;
        // arboard's `set_html` second arg is `Option<impl Into<Cow<str>>>`;
        // map the `Option<String>` into `Option<&str>` explicitly so the
        // generic resolves cleanly.
        let alt_ref: Option<&str> = alt.as_deref();
        cb.set_html(html.as_str(), alt_ref)
            .map_err(|e| IpcError::new("clipboard", "writeHtml", e).into_string())
    }

    /// Read file references from the clipboard. Returns an empty list
    /// when no file-list representation is present.
    pub fn read_files() -> Result<Vec<String>, String> {
        let mut cb = new_cb()?;
        optional_files(cb.get().file_list())
    }

    /// Write file references to the clipboard. Every path goes through the
    /// filesystem policy and capability roots (as `window.startDrag` does);
    /// the authorized, canonical path is what gets written.
    pub fn write_files(paths: Vec<String>) -> Result<(), String> {
        validate_file_list(&paths)?;
        let paths = authorize_file_list(&paths, crate::security::policy::authorize_fs_path)?;
        let mut cb = new_cb()?;
        cb.set()
            .file_list(&paths)
            .map_err(|e| IpcError::new("clipboard", "writeFiles", e).into_string())
    }

    /// Read the clipboard image as a base64 PNG. Returns `null` when
    /// the clipboard holds no image payload.
    pub fn read_image() -> Result<Value, String> {
        read_image_impl()
    }

    /// Write a base64-encoded PNG to the clipboard.
    pub fn write_image(png: String) -> Result<(), String> {
        write_image_impl(&png)
    }

    /// Read RTF from the clipboard. Returns `null` when there is none.
    /// Windows and macOS; rejects on Linux.
    pub fn read_rtf() -> Result<Value, String> {
        Ok(
            formats::read_format(formats::RTF, "readRtf")?.map_or(Value::Null, |bytes| {
                Value::String(String::from_utf8_lossy(&bytes).into_owned())
            }),
        )
    }

    /// Replace the clipboard with RTF and an optional plain-text fallback.
    /// Windows and macOS; rejects on Linux.
    pub fn write_rtf(rtf: String, alt: Option<String>) -> Result<(), String> {
        let mut items = vec![(formats::RTF.to_owned(), rtf.into_bytes())];
        if let Some(alt) = alt {
            items.push(("text/plain".to_owned(), alt.into_bytes()));
        }
        formats::write_formats(&items, "writeRtf")
    }

    /// Formats on the clipboard: MIME types where one is known (`text/plain`,
    /// `text/rtf`, `text/html`, `image/png`...), native names otherwise.
    /// Windows and macOS; rejects on Linux.
    pub fn available_formats() -> Result<Vec<String>, String> {
        formats::available_formats()
    }

    /// Read one format (MIME type, Windows format name or macOS UTI) as
    /// base64 bytes. Returns `null` when the clipboard does not hold it.
    pub fn read_format(format: String) -> Result<Value, String> {
        formats::validate_format(&format, "readFormat")?;
        Ok(formats::read_format(&format, "readFormat")?
            .map_or(Value::Null, |bytes| Value::String(STANDARD.encode(bytes))))
    }

    /// Replace the clipboard with one or more formats at once
    /// (`[{ format, data }]`, `data` base64).
    pub fn write_formats(items: Vec<FormatItem>) -> Result<(), String> {
        let items = decode_format_items(items)?;
        formats::write_formats(&items, "writeFormats")
    }

    /// Empty the clipboard.
    pub fn clear() -> Result<(), String> {
        let mut cb = new_cb()?;
        cb.clear()
            .map_err(|e| IpcError::new("clipboard", "clear", e).into_string())
    }

    /// Begin emitting `clipboard:change` events whenever the
    /// clipboard text payload changes. Polls every `intervalMs`
    /// (default 200, min 50). Reference-counted: each call must be
    /// balanced by a `stopMonitoring`.
    pub fn start_monitoring(interval_ms: Option<u64>) -> Result<(), String> {
        start_monitoring_impl(BACKEND_OWNER, poll_interval(interval_ms))
    }

    /// Release one `startMonitoring` subscription; the watcher stops when
    /// the last is released. No-op when it isn't running.
    pub fn stop_monitoring() {
        stop_monitoring_impl(BACKEND_OWNER);
    }
}

/// One entry of `writeFormats`.
#[derive(Debug, serde::Deserialize)]
pub(crate) struct FormatItem {
    format: String,
    /// Base64 bytes.
    data: String,
}

/// Most formats one `writeFormats` call may set.
const MAX_FORMAT_ITEMS: usize = 32;

/// Validate and decode `writeFormats` items.
fn decode_format_items(items: Vec<FormatItem>) -> Result<Vec<(String, Vec<u8>)>, String> {
    if items.is_empty() || items.len() > MAX_FORMAT_ITEMS {
        return Err(format!(
            "clipboard.writeFormats: pass 1 to {MAX_FORMAT_ITEMS} items, got {}",
            items.len()
        ));
    }
    let mut total = 0_usize;
    items
        .into_iter()
        .map(|item| {
            formats::validate_writable_format(&item.format, "writeFormats")?;
            let bytes = STANDARD.decode(item.data.as_bytes()).map_err(|e| {
                format!(
                    "clipboard.writeFormats: '{}' data is not valid base64: {e}",
                    item.format
                )
            })?;
            total += bytes.len();
            if total > formats::MAX_FORMAT_BYTES {
                return Err(format!(
                    "clipboard.writeFormats: payload exceeds the {}-byte cap",
                    formats::MAX_FORMAT_BYTES
                ));
            }
            Ok((item.format, bytes))
        })
        .collect()
}

/// Renderer-originated `startMonitoring` / `stopMonitoring`, attributed to
/// the calling webview so [`release_webview`] can drop its references when it
/// reloads or closes. `None` when `method` is not a watcher method.
pub(crate) fn dispatch_for_webview(
    method: &str,
    args: &Value,
    webview: &str,
) -> Option<Result<Value, String>> {
    match method {
        "startMonitoring" => Some(
            interval_arg(args)
                .and_then(|interval| start_monitoring_impl(webview, poll_interval(interval)))
                .map(|()| Value::Null),
        ),
        "stopMonitoring" => {
            stop_monitoring_impl(webview);
            Some(Ok(Value::Null))
        },
        _ => None,
    }
}

/// Drop every monitoring reference held by `webview` (reload / close).
pub(crate) fn release_webview(webview: &str) {
    release_and_stop_if_unused(|refs| refs.release_all(webview) > 0);
}

/// Optional `intervalMs` of a renderer `startMonitoring` call.
fn interval_arg(args: &Value) -> Result<Option<u64>, String> {
    match args.get("intervalMs") {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value.as_u64().map(Some).ok_or_else(|| {
            IpcError::new(
                "clipboard",
                "startMonitoring",
                "invalid 'intervalMs': expected an unsigned integer",
            )
            .into_string()
        }),
    }
}

/// Requested poll interval, defaulted and clamped to [`MIN_POLL_MS`].
fn poll_interval(interval_ms: Option<u64>) -> u64 {
    interval_ms.unwrap_or(DEFAULT_POLL_MS).max(MIN_POLL_MS)
}

fn new_cb() -> Result<Clipboard, String> {
    Clipboard::new().map_err(|e| IpcError::new("clipboard", "*", e).into_string())
}

fn optional_html(result: Result<String, arboard::Error>) -> Result<Value, String> {
    match result {
        Ok(html) => Ok(Value::String(html)),
        Err(arboard::Error::ContentNotAvailable) => Ok(Value::Null),
        Err(error) => Err(IpcError::new("clipboard", "readHtml", error).into_string()),
    }
}

fn optional_files(
    result: Result<Vec<std::path::PathBuf>, arboard::Error>,
) -> Result<Vec<String>, String> {
    match result {
        Ok(paths) => Ok(paths
            .into_iter()
            .map(|path| path.to_string_lossy().into_owned())
            .collect()),
        Err(arboard::Error::ContentNotAvailable) => Ok(Vec::new()),
        Err(error) => Err(IpcError::new("clipboard", "readFiles", error).into_string()),
    }
}

const MAX_CLIPBOARD_FILES: usize = 1_024;
const MAX_CLIPBOARD_PATH_BYTES: usize = 32 * 1_024;

/// Resolve every `writeFiles` path through `authorize` (the filesystem
/// policy); the first refusal fails the whole call.
fn authorize_file_list(
    paths: &[String],
    authorize: impl Fn(&str) -> Result<std::path::PathBuf, String>,
) -> Result<Vec<std::path::PathBuf>, String> {
    paths
        .iter()
        .map(|path| {
            authorize(path)
                .map_err(|error| IpcError::new("clipboard", "writeFiles", error).into_string())
        })
        .collect()
}

fn validate_file_list(paths: &[String]) -> Result<(), String> {
    if paths.is_empty() {
        return Err(IpcError::new(
            "clipboard",
            "writeFiles",
            "paths must contain at least one file",
        )
        .into_string());
    }
    if paths.len() > MAX_CLIPBOARD_FILES {
        return Err(IpcError::new(
            "clipboard",
            "writeFiles",
            format!("too many paths (max {MAX_CLIPBOARD_FILES})"),
        )
        .into_string());
    }
    for path in paths {
        if path.is_empty() || path.len() > MAX_CLIPBOARD_PATH_BYTES || path.contains('\0') {
            return Err(IpcError::new(
                "clipboard",
                "writeFiles",
                "paths must be non-empty, NUL-free, and at most 32768 bytes",
            )
            .into_string());
        }
    }
    Ok(())
}

fn read_image_impl() -> Result<Value, String> {
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
        .ok_or_else(|| IpcError::new("clipboard", "readImage", "bad RGBA buffer").into_string())?;
    let mut png = Vec::with_capacity((w as usize) * (h as usize));
    let mut cursor = std::io::Cursor::new(&mut png);
    buffer
        .write_to(&mut cursor, ImageFormat::Png)
        .map_err(|e| {
            IpcError::new("clipboard", "readImage", format!("encode: {e}")).into_string()
        })?;

    Ok(json!({
        "png": STANDARD.encode(&png),
        "width": w,
        "height": h,
    }))
}

/// Hard cap on decoded RGBA bytes for `writeImage`. PNG can compress
/// monochrome / repeated-pattern images at 1000:1 ratios, so a small
/// blob can balloon into gigabytes once decoded — a textbook
/// decompression-bomb DoS. 256 MiB is well above legitimate clipboard
/// images (a 4K screenshot is ~32 MiB RGBA) and well below memory
/// pressure on typical hosts.
const MAX_DECODED_RGBA_BYTES: u64 = 256 * 1024 * 1024;

fn write_image_impl(png_b64: &str) -> Result<(), String> {
    let png_bytes = STANDARD.decode(png_b64).map_err(|e| {
        IpcError::new("clipboard", "writeImage", format!("base64: {e}")).into_string()
    })?;

    // Peek dimensions BEFORE allocating the decoded RGBA buffer so a
    // hostile PNG header advertising 100k x 100k can't OOM us through
    // `to_rgba8`. `image::ImageReader::with_format` parses the header
    // only and exposes `into_dimensions()`.
    let cursor = std::io::Cursor::new(&png_bytes);
    let reader = image::ImageReader::with_format(cursor, ImageFormat::Png);
    let (w, h) = reader.into_dimensions().map_err(|e| {
        IpcError::new("clipboard", "writeImage", format!("dimensions: {e}")).into_string()
    })?;
    let pixels = u64::from(w).saturating_mul(u64::from(h));
    let bytes = pixels.saturating_mul(4);
    if bytes == 0 || bytes > MAX_DECODED_RGBA_BYTES {
        return Err(IpcError::new(
            "clipboard",
            "writeImage",
            format!(
                "decoded RGBA size {bytes} bytes exceeds cap {MAX_DECODED_RGBA_BYTES} \
                 (decompression-bomb guard)"
            ),
        )
        .into_string());
    }

    let dynimg =
        image::load_from_memory_with_format(&png_bytes, ImageFormat::Png).map_err(|e| {
            IpcError::new("clipboard", "writeImage", format!("decode: {e}")).into_string()
        })?;
    let rgba = dynimg.to_rgba8();
    let (w, h) = rgba.dimensions();

    let data = ImageData {
        width: w as usize,
        height: h as usize,
        bytes: rgba.into_raw().into(),
    };
    let mut cb = new_cb()?;
    cb.set_image(data)
        .map_err(|e| IpcError::new("clipboard", "writeImage", e).into_string())
}

// No cross-OS event API for the clipboard — poll on a background thread.
// 200ms is fast enough for typical paste flows and cheap (single text read
// + hash compare). Image changes are detected too, via the same text path:
// arboard returns an error when the clipboard holds a non-text payload, so
// the "last seen" marker just notes transitions.

static LAST_HASH: LazyLock<Mutex<u64>> = LazyLock::new(|| Mutex::new(0));
const DEFAULT_POLL_MS: u64 = 200;
const MIN_POLL_MS: u64 = 50;

struct Monitor {
    running: Arc<AtomicBool>,
    stop: crossbeam_channel::Sender<()>,
    join: Option<JoinHandle<()>>,
}

impl Monitor {
    fn stop(mut self) {
        self.running.store(false, Ordering::Release);
        let _ = self.stop.try_send(());
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

#[derive(Default)]
struct MonitorState {
    shutting_down: bool,
    monitor: Option<Monitor>,
    /// Subscriptions per owner. The monitor is a process-wide singleton, so it
    /// only stops when the last reference is gone.
    refs: WatchRefs,
}

static MONITOR: LazyLock<Mutex<MonitorState>> =
    LazyLock::new(|| Mutex::new(MonitorState::default()));

fn start_monitoring_impl(owner: &str, interval: u64) -> Result<(), String> {
    let mut state = MONITOR.lock();
    if state.shutting_down {
        return Err(
            IpcError::new("clipboard", "startMonitoring", "host is shutting down").into_string(),
        );
    }
    if state.monitor.is_some() {
        state.refs.acquire(owner);
        return Ok(());
    }

    // Seed with the current content so the first tick doesn't fire
    // spuriously. The poll loop uses `get_text().unwrap_or_default()`, so
    // a failed seed MUST degrade to the hash of an empty string (not 0) —
    // otherwise the first successful poll always reports a change.
    let initial = Clipboard::new()
        .and_then(|mut c| c.get_text())
        .unwrap_or_default();
    *LAST_HASH.lock() = hash(initial.as_bytes());

    let running = Arc::new(AtomicBool::new(true));
    let worker_running = Arc::clone(&running);
    let (stop_tx, stop_rx) = crossbeam_channel::bounded(1);
    let join = std::thread::Builder::new()
        .name("tynd-clipboard-monitor".into())
        .spawn(move || poll_loop(&worker_running, &stop_rx, interval))
        .map_err(|error| {
            IpcError::new(
                "clipboard",
                "startMonitoring",
                format!("thread spawn: {error}"),
            )
            .into_string()
        })?;

    state.monitor = Some(Monitor {
        running,
        stop: stop_tx,
        join: Some(join),
    });
    state.refs.acquire(owner);
    Ok(())
}

/// Body of the monitor thread: every `interval` ms, emit `clipboard:change`
/// when the text payload's hash differs from the last one seen.
fn poll_loop(running: &AtomicBool, stop: &crossbeam_channel::Receiver<()>, interval: u64) {
    while running.load(Ordering::Acquire) {
        match stop.recv_timeout(std::time::Duration::from_millis(interval)) {
            Ok(()) | Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => {},
        }
        if !running.load(Ordering::Acquire) {
            break;
        }
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
}

fn stop_monitoring_impl(owner: &str) {
    release_and_stop_if_unused(|refs| refs.release_one(owner));
}

/// Drop references with `release` (which reports whether it dropped any) and
/// stop the monitor once no owner holds one. The monitor thread is joined
/// after the state lock is released.
fn release_and_stop_if_unused(release: impl FnOnce(&mut WatchRefs) -> bool) {
    let monitor = {
        let mut state = MONITOR.lock();
        if !release(&mut state.refs) || state.refs.total() > 0 {
            return;
        }
        state.monitor.take()
    };
    if let Some(monitor) = monitor {
        monitor.stop();
    }
}

pub(crate) fn shutdown() {
    let monitor = {
        let mut state = MONITOR.lock();
        state.shutting_down = true;
        state.monitor.take()
    };
    if let Some(monitor) = monitor {
        monitor.stop();
    }
}

fn hash(bytes: &[u8]) -> u64 {
    // ahash = non-crypto, collision-resistant enough for change detection.
    use std::hash::{BuildHasher as _, Hasher as _};
    let mut h = ahash::RandomState::with_seeds(1, 2, 3, 4).build_hasher();
    h.write(bytes);
    h.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(format: &str, data: &str) -> FormatItem {
        FormatItem {
            format: format.to_owned(),
            data: data.to_owned(),
        }
    }

    #[test]
    fn write_format_items_are_validated() {
        let decoded = decode_format_items(vec![item("text/rtf", "e1xydGYxfQ==")]).unwrap();
        assert_eq!(decoded, vec![("text/rtf".to_owned(), b"{\\rtf1}".to_vec())]);
        let e = decode_format_items(Vec::new()).unwrap_err();
        assert!(e.contains("pass 1 to 32 items"), "{e}");
        let e = decode_format_items(vec![item("x", "!!")]).unwrap_err();
        assert!(e.contains("not valid base64"), "{e}");
        let e = decode_format_items(vec![item("text/html", "")]).unwrap_err();
        assert!(e.contains("writeHtml"), "{e}");
        let many = (0..33).map(|i| item(&format!("f{i}"), "")).collect();
        assert!(decode_format_items(many).is_err());
    }

    #[test]
    fn write_files_paths_go_through_the_fs_policy() {
        let paths = vec!["/allowed/a.txt".to_owned(), "/secret/b.txt".to_owned()];
        let deny_secret = |path: &str| {
            if path.starts_with("/secret") {
                Err(format!("fs: '{path}' is outside the fs scope"))
            } else {
                Ok(std::path::PathBuf::from(path))
            }
        };
        let e = authorize_file_list(&paths, deny_secret).unwrap_err();
        assert!(
            e.contains("writeFiles") && e.contains("outside the fs scope"),
            "{e}"
        );
        let ok = authorize_file_list(&paths[..1], deny_secret).unwrap();
        assert_eq!(ok, vec![std::path::PathBuf::from("/allowed/a.txt")]);
    }

    #[test]
    fn file_reference_formats_are_refused_by_write_formats() {
        let e = decode_format_items(vec![item("CF_HDROP", "")]).unwrap_err();
        assert!(e.contains("use writeFiles"), "{e}");
    }

    #[test]
    fn monitor_refs_are_accounted_per_webview() {
        start_monitoring_impl("wv-a", 50).unwrap();
        start_monitoring_impl("wv-a", 50).unwrap();
        start_monitoring_impl("wv-b", 50).unwrap();
        stop_monitoring_impl("wv-a");
        assert!(MONITOR.lock().monitor.is_some());

        // An unbalanced stop from a webview with no refs cannot steal one.
        stop_monitoring_impl("wv-c");
        assert_eq!(MONITOR.lock().refs.total(), 2);

        // Reload/close of wv-a drops its remaining ref; wv-b keeps the monitor.
        release_webview("wv-a");
        assert_eq!(MONITOR.lock().refs.total(), 1);
        assert!(MONITOR.lock().monitor.is_some());

        // Releasing the last owner stops the monitor; repeats are no-ops.
        release_webview("wv-b");
        assert!(MONITOR.lock().monitor.is_none());
        release_webview("wv-b");
        stop_monitoring_impl("wv-b");
        assert_eq!(MONITOR.lock().refs.total(), 0);
    }
}
