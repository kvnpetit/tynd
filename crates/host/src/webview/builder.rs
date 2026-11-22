//! Builder steps shared by the primary, secondary and child webviews.

use crossbeam_channel::Sender;
use tao::event_loop::EventLoopProxy;
use wry::{http::Request, WebViewBuilder};

use super::{
    handlers::{
        drag_drop_handler, inject_context_labels, inject_runtime_config, page_load_handler,
    },
    navigation::{self, navigation_handler},
};
use crate::{app::UserEvent, ipc::dispatch};
use tynd_protocol::runtime::BackendCall;

use crate::webview::shim;

/// Install what every Tynd webview needs: the init scripts and the IPC,
/// drag-drop, navigation and page-load handlers.
///
/// Init scripts load in this order: context labels, runtime globals, the
/// page-ready hook, then devalue and the IPC shim. Devalue must run before
/// the shim so its encoders/decoders resolve `window.__tynd_dv__` on the
/// first IPC frame. Drag-drop events are reported for `window_label`; the
/// other handlers and IPC routing are keyed by `webview_label`.
pub(crate) fn with_tynd_runtime<'a>(
    builder: WebViewBuilder<'a>,
    window_label: &str,
    webview_label: &str,
    call_tx: &Sender<BackendCall>,
    proxy: &EventLoopProxy<UserEvent>,
) -> WebViewBuilder<'a> {
    let call_tx = call_tx.clone();
    let proxy = proxy.clone();
    let ipc_window_label = window_label.to_owned();
    let ipc_webview_label = webview_label.to_owned();
    builder
        .with_initialization_script(inject_context_labels(window_label, webview_label))
        .with_initialization_script(inject_runtime_config())
        .with_initialization_script(shim::JS_PAGE_READY)
        .with_initialization_script(shim::JS_DEVALUE)
        .with_initialization_script(shim::JS_SHIM)
        .with_ipc_handler(move |req: Request<String>| {
            let origin =
                navigation::ipc_request_origin(req.headers(), req.uri(), &ipc_webview_label);
            dispatch::handle_ipc_body_with_context(
                req.into_body(),
                &ipc_webview_label,
                &ipc_window_label,
                &ipc_webview_label,
                origin,
                &call_tx,
                &proxy,
            );
        })
        .with_drag_drop_handler(drag_drop_handler(window_label))
        .with_navigation_handler(navigation_handler(webview_label))
        .with_on_page_load_handler(page_load_handler(webview_label))
}

/// Enable devtools and the dev-mode flag script when `debug` is set.
pub(crate) fn with_debug_tools(builder: WebViewBuilder<'_>, debug: bool) -> WebViewBuilder<'_> {
    if debug {
        builder
            .with_devtools(true)
            .with_initialization_script(shim::JS_DEV_FLAG)
    } else {
        builder
    }
}

/// Report a web content process termination (typically a dead GPU process)
/// as [`UserEvent::ContextLost`] so the host can reload `label`.
#[cfg(target_os = "macos")]
pub(crate) fn with_context_lost_handler<'a>(
    builder: WebViewBuilder<'a>,
    proxy: &EventLoopProxy<UserEvent>,
    label: String,
) -> WebViewBuilder<'a> {
    use wry::WebViewBuilderExtDarwin;
    let proxy = proxy.clone();
    builder.with_on_web_content_process_terminate_handler(move || {
        let _ = proxy.send_event(UserEvent::ContextLost {
            label: label.clone(),
        });
    })
}
