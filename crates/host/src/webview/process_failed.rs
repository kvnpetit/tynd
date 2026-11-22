//! Renderer process loss: `webview:process-gone` `{ label, reason }`, the
//! event behind the backend's `app.on("render-process-gone")`.
//!
//! - Windows: WebView2 `ProcessFailed` for the webview's own render process
//!   (`crashed`, `unresponsive`) and the browser process (`browser-exited`).
//!   The app decides whether to reload.
//! - macOS: the web content process termination handler (see
//!   `UserEvent::ContextLost`), followed by an automatic reload.
//! - Linux: not reported.

/// Report that the renderer of webview `label` is gone (`reason`: `crashed`,
/// `unresponsive`, `browser-exited`, `frame-crashed`).
pub(crate) fn emit(label: &str, reason: &str) {
    crate::ipc::events::emit(
        "webview:process-gone",
        &serde_json::json!({ "label": label, "reason": reason }),
    );
}

/// Subscribe `webview` to WebView2 `ProcessFailed`.
#[cfg(windows)]
pub(crate) fn watch(webview: &wry::WebView, label: &str) {
    use webview2_com::{
        Microsoft::Web::WebView2::Win32::{
            COREWEBVIEW2_PROCESS_FAILED_KIND,
            COREWEBVIEW2_PROCESS_FAILED_KIND_BROWSER_PROCESS_EXITED,
            COREWEBVIEW2_PROCESS_FAILED_KIND_FRAME_RENDER_PROCESS_EXITED,
            COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_EXITED,
            COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_UNRESPONSIVE,
        },
        ProcessFailedEventHandler,
    };
    use wry::WebViewExtWindows as _;

    let owned_label = label.to_owned();
    let handler = ProcessFailedEventHandler::create(Box::new(move |_sender, args| {
        let label = owned_label.as_str();
        let Some(args) = args else {
            return Ok(());
        };
        let mut kind = COREWEBVIEW2_PROCESS_FAILED_KIND::default();
        // SAFETY: `args` is the live event-args object WebView2 passed to this
        // handler on the UI thread; `kind` is a valid out pointer.
        unsafe { args.ProcessFailedKind(&raw mut kind) }?;
        let reason = match kind {
            COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_EXITED => "crashed",
            COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_UNRESPONSIVE => "unresponsive",
            COREWEBVIEW2_PROCESS_FAILED_KIND_BROWSER_PROCESS_EXITED => "browser-exited",
            // A subframe's renderer: the page itself is still alive.
            COREWEBVIEW2_PROCESS_FAILED_KIND_FRAME_RENDER_PROCESS_EXITED => "frame-crashed",
            // GPU, utility and helper processes are restarted by WebView2.
            _ => return Ok(()),
        };
        crate::tynd_warn!("webview '{label}' render process gone ({reason})");
        emit(label, reason);
        Ok(())
    }));
    let mut token = 0_i64;
    // SAFETY: the ICoreWebView2 of a live webview, on the UI thread that owns
    // it; `token` is a valid out pointer. The handler lives as long as the
    // webview (it is never removed).
    if let Err(error) = unsafe {
        webview
            .webview()
            .add_ProcessFailed(&handler, &raw mut token)
    } {
        crate::tynd_warn!("webview '{label}': ProcessFailed not observed: {error}");
    }
}

/// No-op outside Windows (macOS reports through `ContextLost`).
#[cfg(not(windows))]
pub(crate) fn watch(_webview: &wry::WebView, _label: &str) {}
