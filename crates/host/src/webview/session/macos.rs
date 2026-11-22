//! WKWebView session calls: PDF export (`createPDF`), page capture
//! (`takeSnapshot`) and selective website-data removal. WKWebView has no
//! public audio-mute API and no silent-print path, so those reject.
//!
//! Every call runs on the main thread (window commands are handled by the
//! event loop) and replies from its completion block.

use std::sync::{Arc, Mutex};

use block2::RcBlock;
use objc2::MainThreadMarker;
use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep, NSImage};
use objc2_foundation::{
    NSData, NSDataBase64EncodingOptions, NSDate, NSDictionary, NSError, NSSet, NSString,
};
use objc2_web_kit::{
    WKPDFConfiguration, WKSnapshotConfiguration, WKWebsiteDataStore, WKWebsiteDataTypeCookies,
    WKWebsiteDataTypeDiskCache, WKWebsiteDataTypeFetchCache, WKWebsiteDataTypeIndexedDBDatabases,
    WKWebsiteDataTypeLocalStorage, WKWebsiteDataTypeMemoryCache,
    WKWebsiteDataTypeServiceWorkerRegistrations,
};
use serde_json::Value;
use wry::{WebView, WebViewExtMacOS as _};

use super::{take_reply, unsupported, ClearScope, DataKinds, PdfOptions, Reply};

type Slot = Arc<Mutex<Option<Reply>>>;

fn main_thread(operation: &str) -> Result<MainThreadMarker, String> {
    MainThreadMarker::new().ok_or_else(|| format!("{operation}: must run on the main thread"))
}

fn error_message(operation: &str, error: *mut NSError) -> String {
    // SAFETY: WebKit passes a valid NSError or null.
    match unsafe { error.as_ref() } {
        Some(error) => format!("{operation}: {}", error.localizedDescription()),
        None => format!("{operation}: the engine returned no data"),
    }
}

fn base64(data: &NSData) -> Value {
    Value::String(
        data.base64EncodedStringWithOptions(NSDataBase64EncodingOptions::empty())
            .to_string(),
    )
}

/// WKWebView renders the whole page as one continuous PDF page: paper size,
/// margins, orientation and scale do not apply.
pub(super) fn print_to_pdf(webview: &WebView, _options: &PdfOptions, reply: Reply) {
    let mtm = match main_thread("printToPdf") {
        Ok(mtm) => mtm,
        Err(error) => return reply.send(Err(error)),
    };
    let slot: Slot = Arc::new(Mutex::new(Some(reply)));
    let block = RcBlock::new(move |data: *mut NSData, error: *mut NSError| {
        let Some(reply) = take_reply(&slot) else {
            return;
        };
        // SAFETY: WebKit passes a valid NSData or null.
        let result = match unsafe { data.as_ref() } {
            Some(data) => Ok(base64(data)),
            None => Err(error_message("printToPdf", error)),
        };
        reply.send(result);
    });
    let wk = webview.webview();
    // SAFETY: main thread; the configuration and block are retained by the
    // call until the completion handler runs.
    unsafe {
        let configuration = WKPDFConfiguration::new(mtm);
        wk.createPDFWithConfiguration_completionHandler(Some(&configuration), &block);
    }
}

fn image_png(image: &NSImage) -> Result<Value, String> {
    let tiff = image
        .TIFFRepresentation()
        .ok_or("capturePage: the snapshot is empty")?;
    let bitmap = NSBitmapImageRep::imageRepWithData(&tiff)
        .ok_or("capturePage: the snapshot could not be decoded")?;
    // SAFETY: an empty property dictionary is valid for PNG encoding.
    let png = unsafe {
        bitmap.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new())
    }
    .ok_or("capturePage: PNG encoding failed")?;
    Ok(base64(&png))
}

/// Capture the visible viewport.
pub(super) fn capture_page(webview: &WebView, reply: Reply) {
    let mtm = match main_thread("capturePage") {
        Ok(mtm) => mtm,
        Err(error) => return reply.send(Err(error)),
    };
    let slot: Slot = Arc::new(Mutex::new(Some(reply)));
    let block = RcBlock::new(move |image: *mut NSImage, error: *mut NSError| {
        let Some(reply) = take_reply(&slot) else {
            return;
        };
        // SAFETY: WebKit passes a valid NSImage or null.
        let result = match unsafe { image.as_ref() } {
            Some(image) => image_png(image),
            None => Err(error_message("capturePage", error)),
        };
        reply.send(result);
    });
    let wk = webview.webview();
    // SAFETY: main thread; the configuration and block are retained by the
    // call until the completion handler runs.
    unsafe {
        let configuration = WKSnapshotConfiguration::new(mtm);
        wk.takeSnapshotWithConfiguration_completionHandler(Some(&configuration), &block);
    }
}

pub(super) fn set_muted(_webview: &WebView, _muted: bool) -> Result<(), String> {
    Err(unsupported("setAudioMuted"))
}

pub(super) fn is_muted(_webview: &WebView) -> Result<bool, String> {
    Err(unsupported("isAudioMuted"))
}

pub(super) fn print_silently(_webview: &WebView, _printer: Option<&str>, reply: Reply) {
    reply.send(Err(format!(
        "{} (use print() for the print dialog)",
        unsupported("print({ silent: true })")
    )));
}

pub(super) fn printers() -> Result<Value, String> {
    Err(unsupported("getPrinters"))
}

/// WKWebsiteDataStore types of the selected categories. `cache` covers the
/// disk, memory and fetch (Cache API) caches.
fn website_data_types(kinds: DataKinds) -> Vec<&'static NSString> {
    // SAFETY: WebKit's exported data-type constants are immutable NSStrings.
    let mapping: [(u8, &[&'static NSString]); 5] = unsafe {
        [
            (DataKinds::COOKIES, &[WKWebsiteDataTypeCookies]),
            (
                DataKinds::CACHE,
                &[
                    WKWebsiteDataTypeDiskCache,
                    WKWebsiteDataTypeMemoryCache,
                    WKWebsiteDataTypeFetchCache,
                ],
            ),
            (DataKinds::LOCAL_STORAGE, &[WKWebsiteDataTypeLocalStorage]),
            (
                DataKinds::INDEXED_DB,
                &[WKWebsiteDataTypeIndexedDBDatabases],
            ),
            (
                DataKinds::SERVICE_WORKERS,
                &[WKWebsiteDataTypeServiceWorkerRegistrations],
            ),
        ]
    };
    mapping
        .into_iter()
        .filter(|(kind, _)| kinds.has(*kind))
        .flat_map(|(_, types)| types.iter().copied())
        .collect()
}

pub(super) fn clear_data(webview: &WebView, scope: ClearScope, reply: Reply) {
    let mtm = match main_thread("clearBrowsingData") {
        Ok(mtm) => mtm,
        Err(error) => return reply.send(Err(error)),
    };
    let slot: Slot = Arc::new(Mutex::new(Some(reply)));
    let block = RcBlock::new(move || {
        if let Some(reply) = take_reply(&slot) {
            reply.send(Ok(Value::Null));
        }
    });
    let wk = webview.webview();
    // SAFETY: main thread; the store, type set, date and block are retained
    // by the call until the completion handler runs.
    unsafe {
        let types = match scope {
            ClearScope::Everything => WKWebsiteDataStore::allWebsiteDataTypes(mtm),
            ClearScope::Only(kinds) => NSSet::from_slice(&website_data_types(kinds)),
        };
        let store = wk.configuration().websiteDataStore();
        let since = NSDate::distantPast();
        store.removeDataOfTypes_modifiedSince_completionHandler(&types, &since, &block);
    }
}
