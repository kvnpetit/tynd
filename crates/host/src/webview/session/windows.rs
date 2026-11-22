//! WebView2 calls wry does not wrap: DevTools-protocol PDF export and page
//! capture, audio mute (`ICoreWebView2_8`), selective browsing-data removal
//! (`ICoreWebView2Profile2`), silent printing (`ICoreWebView2_16`) and
//! printer enumeration (winspool).
//!
//! `CallDevToolsProtocolMethod` is a regular WebView2 API: it does not need
//! the DevTools window, and `AreDevToolsEnabled = false` (release builds) only
//! hides the DevTools UI from the user, so PDF export and capture work in
//! release builds too.

use serde_json::{json, Value};
use webview2_com::{
    CallDevToolsProtocolMethodCompletedHandler, ClearBrowsingDataCompletedHandler,
    Microsoft::Web::WebView2::Win32::{
        ICoreWebView2Environment6, ICoreWebView2PrintSettings2, ICoreWebView2Profile2,
        ICoreWebView2_13, ICoreWebView2_16, ICoreWebView2_8, COREWEBVIEW2_BROWSING_DATA_KINDS,
        COREWEBVIEW2_BROWSING_DATA_KINDS_CACHE_STORAGE, COREWEBVIEW2_BROWSING_DATA_KINDS_COOKIES,
        COREWEBVIEW2_BROWSING_DATA_KINDS_DISK_CACHE, COREWEBVIEW2_BROWSING_DATA_KINDS_INDEXED_DB,
        COREWEBVIEW2_BROWSING_DATA_KINDS_LOCAL_STORAGE,
        COREWEBVIEW2_BROWSING_DATA_KINDS_SERVICE_WORKERS, COREWEBVIEW2_PRINT_STATUS,
        COREWEBVIEW2_PRINT_STATUS_PRINTER_UNAVAILABLE, COREWEBVIEW2_PRINT_STATUS_SUCCEEDED,
    },
    PrintCompletedHandler,
};
use windows_core::{Interface as _, BOOL, HSTRING};
use wry::{WebView, WebViewExtWindows as _};

use super::{cdp_data, take_reply, ClearScope, DataKinds, PdfOptions, Reply};

pub(super) fn print_to_pdf(webview: &WebView, options: &PdfOptions, reply: Reply) {
    devtools_data(
        webview,
        "printToPdf",
        "Page.printToPDF",
        &options.to_cdp(),
        reply,
    );
}

pub(super) fn capture_page(webview: &WebView, reply: Reply) {
    let params = json!({ "format": "png" });
    devtools_data(
        webview,
        "capturePage",
        "Page.captureScreenshot",
        &params,
        reply,
    );
}

/// Run a DevTools-protocol `cdp_method` and reply with the base64 `data`
/// field of its result.
fn devtools_data(
    webview: &WebView,
    operation: &'static str,
    cdp_method: &str,
    params: &Value,
    reply: Reply,
) {
    let core = webview.webview();
    let slot = std::sync::Arc::new(std::sync::Mutex::new(Some(reply)));
    let callback_slot = std::sync::Arc::clone(&slot);
    let handler = CallDevToolsProtocolMethodCompletedHandler::create(Box::new(
        move |result: windows_core::Result<()>, json: String| {
            if let Some(reply) = take_reply(&callback_slot) {
                reply.send(
                    result
                        .map_err(|error| format!("{operation}: {}", error.message()))
                        .and_then(|()| cdp_data(operation, &json)),
                );
            }
            Ok(())
        },
    ));
    let method = HSTRING::from(cdp_method);
    let params = HSTRING::from(params.to_string());
    // SAFETY: `core` is the live ICoreWebView2 of `webview`, called on the UI
    // thread that owns it; the HSTRING arguments outlive the call.
    let started = unsafe { core.CallDevToolsProtocolMethod(&method, &params, &handler) };
    if let Err(error) = started {
        if let Some(reply) = take_reply(&slot) {
            reply.send(Err(format!("{operation}: {}", error.message())));
        }
    }
}

fn webview8(webview: &WebView, operation: &str) -> Result<ICoreWebView2_8, String> {
    webview
        .webview()
        .cast::<ICoreWebView2_8>()
        .map_err(|_| format!("{operation}: needs a newer WebView2 Runtime"))
}

pub(super) fn set_muted(webview: &WebView, muted: bool) -> Result<(), String> {
    let core = webview8(webview, "setAudioMuted")?;
    // SAFETY: live interface, UI thread.
    unsafe { core.SetIsMuted(muted) }.map_err(|error| format!("setAudioMuted: {}", error.message()))
}

pub(super) fn is_muted(webview: &WebView) -> Result<bool, String> {
    let core = webview8(webview, "isAudioMuted")?;
    let mut muted = BOOL::default();
    // SAFETY: live interface, UI thread; `muted` is a valid out pointer.
    unsafe { core.IsMuted(&raw mut muted) }
        .map_err(|error| format!("isAudioMuted: {}", error.message()))?;
    Ok(muted.as_bool())
}

/// WebView2 kinds of the selected categories. `cache` covers the HTTP disk
/// cache and the Cache API storage.
fn browsing_data_kinds(kinds: DataKinds) -> COREWEBVIEW2_BROWSING_DATA_KINDS {
    let mapping = [
        (
            DataKinds::COOKIES,
            COREWEBVIEW2_BROWSING_DATA_KINDS_COOKIES.0,
        ),
        (
            DataKinds::CACHE,
            COREWEBVIEW2_BROWSING_DATA_KINDS_DISK_CACHE.0
                | COREWEBVIEW2_BROWSING_DATA_KINDS_CACHE_STORAGE.0,
        ),
        (
            DataKinds::LOCAL_STORAGE,
            COREWEBVIEW2_BROWSING_DATA_KINDS_LOCAL_STORAGE.0,
        ),
        (
            DataKinds::INDEXED_DB,
            COREWEBVIEW2_BROWSING_DATA_KINDS_INDEXED_DB.0,
        ),
        (
            DataKinds::SERVICE_WORKERS,
            COREWEBVIEW2_BROWSING_DATA_KINDS_SERVICE_WORKERS.0,
        ),
    ];
    COREWEBVIEW2_BROWSING_DATA_KINDS(
        mapping
            .iter()
            .filter(|(kind, _)| kinds.has(*kind))
            .fold(0, |bits, (_, native)| bits | native),
    )
}

pub(super) fn clear_data(webview: &WebView, scope: ClearScope, reply: Reply) {
    let profile = webview
        .webview()
        .cast::<ICoreWebView2_13>()
        // SAFETY: live interface, UI thread.
        .and_then(|core| unsafe { core.Profile() })
        .and_then(|profile| profile.cast::<ICoreWebView2Profile2>());
    let Ok(profile) = profile else {
        return reply.send(Err(
            "clearBrowsingData: needs a newer WebView2 Runtime".into()
        ));
    };
    let slot = std::sync::Arc::new(std::sync::Mutex::new(Some(reply)));
    let callback_slot = std::sync::Arc::clone(&slot);
    let handler = ClearBrowsingDataCompletedHandler::create(Box::new(
        move |result: windows_core::Result<()>| {
            if let Some(reply) = take_reply(&callback_slot) {
                reply.send(
                    result
                        .map(|()| Value::Null)
                        .map_err(|error| format!("clearBrowsingData: {}", error.message())),
                );
            }
            Ok(())
        },
    ));
    // SAFETY: live interface, UI thread; the handler is reference counted.
    let started = unsafe {
        match scope {
            ClearScope::Everything => profile.ClearBrowsingDataAll(&handler),
            ClearScope::Only(kinds) => {
                profile.ClearBrowsingData(browsing_data_kinds(kinds), &handler)
            },
        }
    };
    if let Err(error) = started {
        if let Some(reply) = take_reply(&slot) {
            reply.send(Err(format!("clearBrowsingData: {}", error.message())));
        }
    }
}

fn print_status_result(
    status: COREWEBVIEW2_PRINT_STATUS,
    printer: Option<&str>,
) -> Result<Value, String> {
    if status == COREWEBVIEW2_PRINT_STATUS_SUCCEEDED {
        Ok(Value::Null)
    } else if status == COREWEBVIEW2_PRINT_STATUS_PRINTER_UNAVAILABLE {
        Err(printer.map_or_else(
            || "print: no default printer is available".to_owned(),
            |name| format!("print: printer '{name}' is not available (see getPrinters())"),
        ))
    } else {
        Err("print: the print job failed".into())
    }
}

/// Print without a dialog to `printer` (or the default printer).
pub(super) fn print_silently(webview: &WebView, printer: Option<&str>, reply: Reply) {
    let prepared = (|| {
        let core = webview.webview().cast::<ICoreWebView2_16>()?;
        let environment = webview.environment().cast::<ICoreWebView2Environment6>()?;
        // SAFETY: live interfaces, UI thread.
        let settings = unsafe { environment.CreatePrintSettings() }?;
        if let Some(name) = printer {
            let named = settings.cast::<ICoreWebView2PrintSettings2>()?;
            // SAFETY: live interface, UI thread; the HSTRING outlives the call.
            unsafe { named.SetPrinterName(&HSTRING::from(name)) }?;
        }
        windows_core::Result::Ok((core, settings))
    })();
    let Ok((core, settings)) = prepared else {
        return reply.send(Err(
            "print: silent printing needs a newer WebView2 Runtime".into()
        ));
    };
    let printer = printer.map(str::to_owned);
    let slot = std::sync::Arc::new(std::sync::Mutex::new(Some(reply)));
    let callback_slot = std::sync::Arc::clone(&slot);
    let handler = PrintCompletedHandler::create(Box::new(
        move |result: windows_core::Result<()>, status: COREWEBVIEW2_PRINT_STATUS| {
            if let Some(reply) = take_reply(&callback_slot) {
                reply.send(
                    result
                        .map_err(|error| format!("print: {}", error.message()))
                        .and_then(|()| print_status_result(status, printer.as_deref())),
                );
            }
            Ok(())
        },
    ));
    // SAFETY: live interfaces, UI thread; the handler is reference counted.
    let started = unsafe { core.Print(&settings, &handler) };
    if let Err(error) = started {
        if let Some(reply) = take_reply(&slot) {
            reply.send(Err(format!("print: {}", error.message())));
        }
    }
}

/// Read a NUL-terminated UTF-16 string.
///
/// # Safety
/// `ptr` must be null or point to a NUL-terminated UTF-16 string.
unsafe fn wide_string(ptr: *const u16) -> Option<String> {
    if ptr.is_null() {
        return None;
    }
    let mut len = 0;
    // SAFETY: the caller guarantees a NUL terminator.
    while unsafe { *ptr.add(len) } != 0 {
        len += 1;
    }
    // SAFETY: `len` code units were just read.
    Some(String::from_utf16_lossy(unsafe {
        std::slice::from_raw_parts(ptr, len)
    }))
}

fn default_printer() -> Option<String> {
    use windows_sys::Win32::Graphics::Printing::GetDefaultPrinterW;

    let mut len = 0u32;
    // SAFETY: size query with a null buffer; `len` is a valid out pointer.
    unsafe { GetDefaultPrinterW(std::ptr::null_mut(), &raw mut len) };
    if len == 0 {
        return None;
    }
    let mut buffer = vec![0u16; len as usize];
    // SAFETY: `buffer` holds `len` UTF-16 code units.
    let ok = unsafe { GetDefaultPrinterW(buffer.as_mut_ptr(), &raw mut len) };
    // SAFETY: on success the buffer holds a NUL-terminated name.
    (ok != 0).then(|| unsafe { wide_string(buffer.as_ptr()) })?
}

/// Installed local and connected printers, with the default one flagged.
pub(super) fn printers() -> Result<Value, String> {
    use windows_sys::Win32::Graphics::Printing::{
        EnumPrintersW, PRINTER_ENUM_CONNECTIONS, PRINTER_ENUM_LOCAL, PRINTER_INFO_4W,
    };

    const FLAGS: u32 = PRINTER_ENUM_LOCAL | PRINTER_ENUM_CONNECTIONS;
    let mut needed = 0u32;
    let mut count = 0u32;
    // SAFETY: size query with a null buffer; the out pointers are valid.
    unsafe {
        EnumPrintersW(
            FLAGS,
            std::ptr::null(),
            4,
            std::ptr::null_mut(),
            0,
            &raw mut needed,
            &raw mut count,
        )
    };
    // A printer may be added between the two calls: retry with the new size.
    let mut attempts = 0;
    let (buffer, count) = loop {
        if needed == 0 {
            return Ok(json!([]));
        }
        // u64 storage keeps the PRINTER_INFO_4W array aligned.
        let mut buffer = vec![0u64; (needed as usize).div_ceil(8)];
        // SAFETY: `buffer` holds at least `needed` bytes; out pointers valid.
        let ok = unsafe {
            EnumPrintersW(
                FLAGS,
                std::ptr::null(),
                4,
                buffer.as_mut_ptr().cast(),
                needed,
                &raw mut needed,
                &raw mut count,
            )
        };
        if ok != 0 {
            break (buffer, count);
        }
        attempts += 1;
        if attempts == 3 {
            return Err(format!("getPrinters: {}", std::io::Error::last_os_error()));
        }
    };
    let default = default_printer();
    // SAFETY: on success the buffer starts with `count` PRINTER_INFO_4W
    // records whose strings point into the same buffer.
    let infos = unsafe {
        std::slice::from_raw_parts(buffer.as_ptr().cast::<PRINTER_INFO_4W>(), count as usize)
    };
    let printers: Vec<Value> = infos
        .iter()
        // SAFETY: `pPrinterName` points to a NUL-terminated name in `buffer`.
        .filter_map(|info| unsafe { wide_string(info.pPrinterName) })
        .map(|name| {
            let is_default = default.as_deref() == Some(name.as_str());
            json!({ "name": name, "isDefault": is_default })
        })
        .collect();
    Ok(Value::Array(printers))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn categories_map_to_webview2_kinds() {
        let kinds = browsing_data_kinds(DataKinds(DataKinds::COOKIES | DataKinds::CACHE));
        assert_eq!(
            kinds.0,
            COREWEBVIEW2_BROWSING_DATA_KINDS_COOKIES.0
                | COREWEBVIEW2_BROWSING_DATA_KINDS_DISK_CACHE.0
                | COREWEBVIEW2_BROWSING_DATA_KINDS_CACHE_STORAGE.0
        );
        assert_eq!(
            browsing_data_kinds(DataKinds(DataKinds::SERVICE_WORKERS)).0,
            COREWEBVIEW2_BROWSING_DATA_KINDS_SERVICE_WORKERS.0
        );
    }

    #[test]
    fn print_status_maps_to_errors() {
        assert_eq!(
            print_status_result(COREWEBVIEW2_PRINT_STATUS_SUCCEEDED, None).unwrap(),
            Value::Null
        );
        let error = print_status_result(
            COREWEBVIEW2_PRINT_STATUS_PRINTER_UNAVAILABLE,
            Some("Office"),
        )
        .unwrap_err();
        assert!(error.contains("'Office' is not available"), "{error}");
        let error =
            print_status_result(COREWEBVIEW2_PRINT_STATUS_PRINTER_UNAVAILABLE, None).unwrap_err();
        assert!(error.contains("no default printer"), "{error}");
    }

    #[test]
    fn wide_strings_are_read_up_to_the_terminator() {
        let wide: Vec<u16> = "HP LaserJet\0junk".encode_utf16().collect();
        // SAFETY: the vector contains a NUL terminator.
        assert_eq!(
            unsafe { wide_string(wide.as_ptr()) }.as_deref(),
            Some("HP LaserJet")
        );
        // SAFETY: null is accepted.
        assert_eq!(unsafe { wide_string(std::ptr::null()) }, None);
    }
}
