//! WebKitGTK session calls: snapshot capture, print-to-file PDF export,
//! silent printing, audio mute and selective website-data removal.

use std::{cell::RefCell, rc::Rc};

use gtk::{gdk, gio, glib};
use serde_json::Value;
use webkit2gtk::{
    PrintOperation, PrintOperationExt as _, SnapshotOptions, SnapshotRegion, WebViewExt as _,
    WebsiteDataManagerExtManual as _, WebsiteDataTypes,
};
use wry::{WebView, WebViewExtUnix as _};

use super::{unsupported, ClearScope, DataKinds, PdfOptions, Reply};

/// GTK's file printer. Its name is translated with the `gtk30` domain, so
/// look it up the way GTK registers it.
fn file_printer_name() -> glib::GString {
    glib::dgettext(Some("gtk30"), "Print to File")
}

pub(super) fn capture_page(webview: &WebView, reply: Reply) {
    webview.webview().snapshot(
        SnapshotRegion::Visible,
        SnapshotOptions::NONE,
        None::<&gio::Cancellable>,
        move |result| {
            reply.send(
                result
                    .map_err(|error| format!("capturePage: {error}"))
                    .and_then(surface_png),
            );
        },
    );
}

/// Encode a snapshot surface as base64 PNG.
fn surface_png(surface: gtk::cairo::Surface) -> Result<Value, String> {
    let image = gtk::cairo::ImageSurface::try_from(surface)
        .map_err(|_| "capturePage: the engine returned an unexpected surface".to_owned())?;
    let pixbuf = gdk::pixbuf_get_from_surface(&image, 0, 0, image.width(), image.height())
        .ok_or("capturePage: the snapshot is empty")?;
    let png = pixbuf
        .save_to_bufferv("png", &[])
        .map_err(|error| format!("capturePage: {error}"))?;
    Ok(Value::String(glib::base64_encode(&png).to_string()))
}

/// Paper, orientation and margins of `options`.
fn page_setup(options: &PdfOptions) -> gtk::PageSetup {
    let setup = gtk::PageSetup::new();
    setup.set_orientation(if options.landscape {
        gtk::PageOrientation::Landscape
    } else {
        gtk::PageOrientation::Portrait
    });
    if let Some((width, height)) = options.page_size {
        let paper = gtk::PaperSize::new_custom("tynd", "tynd", width, height, gtk::Unit::Inch);
        setup.set_paper_size(&paper);
    }
    if let Some([top, right, bottom, left]) = options.margins {
        setup.set_top_margin(top, gtk::Unit::Inch);
        setup.set_right_margin(right, gtk::Unit::Inch);
        setup.set_bottom_margin(bottom, gtk::Unit::Inch);
        setup.set_left_margin(left, gtk::Unit::Inch);
    }
    setup
}

/// State kept alive until a print operation reports back. Holding the
/// operation here (and dropping it on completion) keeps it alive while
/// WebKit prints without leaking it afterwards.
struct PendingPrint {
    reply: Reply,
    _operation: PrintOperation,
    /// PDF destination of `printToPdf`; deleted when dropped.
    output: Option<tempfile::NamedTempFile>,
}

type PendingSlot = Rc<RefCell<Option<PendingPrint>>>;

/// Start `operation` without a dialog. `finish` turns the finished
/// operation into the reply.
fn run_print(
    operation: PrintOperation,
    reply: Reply,
    output: Option<tempfile::NamedTempFile>,
    error_prefix: &'static str,
    finish: fn(&PendingPrint) -> Result<Value, String>,
) {
    let slot: PendingSlot = Rc::new(RefCell::new(Some(PendingPrint {
        reply,
        _operation: operation.clone(),
        output,
    })));
    let failed_slot = Rc::clone(&slot);
    operation.connect_failed(move |_, error| {
        if let Some(pending) = failed_slot.borrow_mut().take() {
            pending.reply.send(Err(format!("{error_prefix}: {error}")));
        }
    });
    let finished_slot = Rc::clone(&slot);
    operation.connect_finished(move |_| {
        // `failed` fires before `finished`: the slot is empty then.
        let pending = finished_slot.borrow_mut().take();
        if let Some(pending) = pending {
            let result = finish(&pending);
            // The operation and the temporary file drop with `pending`.
            pending.reply.send(result);
        }
    });
    operation.print();
}

fn read_pdf(pending: &PendingPrint) -> Result<Value, String> {
    let output = pending
        .output
        .as_ref()
        .ok_or("printToPdf: no output file")?;
    let bytes = std::fs::read(output.path()).map_err(|error| format!("printToPdf: {error}"))?;
    if bytes.is_empty() {
        return Err("printToPdf: the engine produced no PDF".into());
    }
    Ok(Value::String(glib::base64_encode(&bytes).to_string()))
}

pub(super) fn print_to_pdf(webview: &WebView, options: &PdfOptions, reply: Reply) {
    // Private (0600) temporary file the GTK file printer overwrites.
    let output = match tempfile::Builder::new()
        .prefix("tynd-print-")
        .suffix(".pdf")
        .tempfile()
    {
        Ok(output) => output,
        Err(error) => return reply.send(Err(format!("printToPdf: {error}"))),
    };
    let uri = match glib::filename_to_uri(output.path(), None) {
        Ok(uri) => uri,
        Err(error) => return reply.send(Err(format!("printToPdf: {error}"))),
    };
    let settings = gtk::PrintSettings::new();
    settings.set_printer(&file_printer_name());
    settings.set(gtk::PRINT_SETTINGS_OUTPUT_FILE_FORMAT, Some("pdf"));
    settings.set(gtk::PRINT_SETTINGS_OUTPUT_URI, Some(uri.as_str()));
    settings.set_scale(options.scale * 100.0);
    let operation = PrintOperation::new(&webview.webview());
    operation.set_print_settings(&settings);
    operation.set_page_setup(&page_setup(options));
    run_print(operation, reply, Some(output), "printToPdf", read_pdf);
}

pub(super) fn print_silently(webview: &WebView, printer: Option<&str>, reply: Reply) {
    let operation = PrintOperation::new(&webview.webview());
    if let Some(name) = printer {
        let settings = gtk::PrintSettings::new();
        settings.set_printer(name);
        operation.set_print_settings(&settings);
    }
    run_print(operation, reply, None, "print", |_| Ok(Value::Null));
}

pub(super) fn printers() -> Result<Value, String> {
    // GTK enumerates printers through gtkunixprint, which gtk-rs does not bind.
    Err(unsupported("getPrinters"))
}

pub(super) fn set_muted(webview: &WebView, muted: bool) -> Result<(), String> {
    webview.webview().set_is_muted(muted);
    Ok(())
}

pub(super) fn is_muted(webview: &WebView) -> Result<bool, String> {
    Ok(webview.webview().is_muted())
}

/// WebKitGTK data types of the selected categories. `cache` covers the disk
/// and memory caches and the Cache API storage.
fn website_data_types(kinds: DataKinds) -> WebsiteDataTypes {
    let mapping = [
        (DataKinds::COOKIES, WebsiteDataTypes::COOKIES),
        (
            DataKinds::CACHE,
            WebsiteDataTypes::DISK_CACHE
                | WebsiteDataTypes::MEMORY_CACHE
                | WebsiteDataTypes::DOM_CACHE,
        ),
        (DataKinds::LOCAL_STORAGE, WebsiteDataTypes::LOCAL_STORAGE),
        (DataKinds::INDEXED_DB, WebsiteDataTypes::INDEXEDDB_DATABASES),
        (
            DataKinds::SERVICE_WORKERS,
            WebsiteDataTypes::SERVICE_WORKER_REGISTRATIONS,
        ),
    ];
    mapping
        .iter()
        .filter(|(kind, _)| kinds.has(*kind))
        .fold(WebsiteDataTypes::empty(), |types, (_, native)| {
            types | *native
        })
}

pub(super) fn clear_data(webview: &WebView, scope: ClearScope, reply: Reply) {
    let Some(manager) = webview.webview().website_data_manager() else {
        return reply.send(Err(
            "clearBrowsingData: the webview has no data store".into()
        ));
    };
    let types = match scope {
        ClearScope::Everything => WebsiteDataTypes::ALL,
        ClearScope::Only(kinds) => website_data_types(kinds),
    };
    manager.clear(
        types,
        glib::TimeSpan::from_seconds(0),
        None::<&gio::Cancellable>,
        move |result| {
            reply.send(
                result
                    .map(|()| Value::Null)
                    .map_err(|error| format!("clearBrowsingData: {error}")),
            );
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn categories_map_to_webkit_data_types() {
        assert_eq!(
            website_data_types(DataKinds(DataKinds::COOKIES | DataKinds::LOCAL_STORAGE)),
            WebsiteDataTypes::COOKIES | WebsiteDataTypes::LOCAL_STORAGE
        );
        let cache = website_data_types(DataKinds(DataKinds::CACHE));
        assert!(cache.contains(WebsiteDataTypes::DISK_CACHE | WebsiteDataTypes::MEMORY_CACHE));
        assert!(!cache.contains(WebsiteDataTypes::COOKIES));
    }
}
