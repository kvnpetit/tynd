//! Builder steps shared by the primary, secondary and child webviews.

use std::sync::Arc;

use crossbeam_channel::Sender;
use serde_json::Value;
use tao::event_loop::EventLoopProxy;
use wry::{http::Request, WebViewBuilder};

use super::{
    handlers::{
        drag_drop_handler, inject_context_labels, inject_runtime_config, is_app_local_webview,
        page_load_handler,
    },
    navigation::{self, navigation_handler},
};
use crate::{app::UserEvent, ipc::dispatch};
use tynd_protocol::runtime::BackendCall;

use crate::webview::shim::{
    self,
    tokens::{self, IpcTokens, TokenKind},
};

/// Install what every Tynd webview needs: the init scripts and the IPC,
/// drag-drop, navigation and page-load handlers.
///
/// Init scripts load in this order: context labels, runtime globals, then
/// devalue and the IPC shim (which also sends the page-ready signal). Devalue
/// must run before the shim so its encoders/decoders resolve
/// `window.__tynd_dv__` on the first IPC frame.
///
/// The shim prefixes every IPC message with one of two per-webview secret
/// tokens and the IPC handler drops messages with neither (see
/// [`shim::tokens`]). WebKit (macOS, Linux) exposes the raw script-message
/// handler to every frame and WebKitGTK cannot tell the host which frame
/// posted, so without them a cross-origin iframe could post as the main frame.
///
/// - The *remote* token is baked into the init script, so every document,
///   remote ones included, holds it: a message carrying it gets remote-origin
///   privileges at most (`null`, which matches no capability, when the
///   tracked main frame is app content).
/// - The *app* token is never in a script. The app document's shim asks for
///   it and the host answers, in the main frame, only while that frame shows
///   app content; a message carrying it is dropped unless the main frame is
///   app content. It is re-minted on every remote commit.
///
/// wry 0.57 cannot replace an init script after the webview is built, hence
/// the out-of-band hand-off rather than per-navigation scripts.
///
/// Drag-drop events are reported for `window_label`; the other handlers and
/// IPC routing are keyed by `webview_label`.
pub(crate) fn with_tynd_runtime<'a>(
    builder: WebViewBuilder<'a>,
    window_label: &str,
    webview_label: &str,
    call_tx: &Sender<BackendCall>,
    proxy: &EventLoopProxy<UserEvent>,
) -> WebViewBuilder<'a> {
    let page_call_tx = call_tx.clone();
    let call_tx = call_tx.clone();
    let ipc_proxy = proxy.clone();
    let ipc_window_label = window_label.to_owned();
    let ipc_webview_label = webview_label.to_owned();
    let tokens = IpcTokens::new();
    let shim_script = shim::ipc_shim(tokens.remote(), navigation::dev_origin().as_deref());
    let ipc_tokens = Arc::clone(&tokens);
    let commit_label = webview_label.to_owned();
    let commit_proxy = proxy.clone();
    builder
        .with_initialization_script(inject_context_labels(window_label, webview_label))
        .with_initialization_script(inject_runtime_config())
        .with_initialization_script(shim::JS_DEVALUE)
        .with_initialization_script(shim_script)
        .with_initialization_script(crate::api::menu::JS_CONTEXT_MENU)
        .with_initialization_script(shim::JS_FRAME)
        .with_ipc_handler(move |req: Request<String>| {
            let label = ipc_webview_label.as_str();
            let origin = navigation::ipc_request_origin(req.headers(), req.uri(), label);
            let mut body = req.into_body();
            let origin = match ipc_tokens.take(&mut body) {
                None => {
                    tracing::debug!(
                        target: "tynd",
                        "webview {label}: dropped an IPC message without a webview token"
                    );
                    return;
                },
                Some(TokenKind::App) => {
                    if !is_app_local_webview(label) {
                        tracing::debug!(
                            target: "tynd",
                            "webview {label}: dropped an app-token IPC message, the main frame is not app content"
                        );
                        return;
                    }
                    origin
                },
                Some(TokenKind::Remote) => {
                    if let Some(request_id) = tokens::app_token_request(&body) {
                        answer_app_token_request(&ipc_tokens, label, request_id, &ipc_proxy);
                        return;
                    }
                    remote_token_origin(origin)
                },
            };
            dispatch::handle_ipc_body_with_context(
                body,
                label,
                &ipc_window_label,
                label,
                origin,
                &call_tx,
                &ipc_proxy,
            );
        })
        .with_drag_drop_handler(drag_drop_handler(window_label))
        .with_navigation_handler(navigation_handler(webview_label))
        .with_on_page_load_handler(page_load_handler(webview_label, &page_call_tx, move || {
            on_document_committed(&tokens, &commit_label, &commit_proxy);
        }))
}

/// Caller origin of a remote-token message: never app-local. A frame the
/// host would classify as app content (or as the app-local main frame) did
/// not prove it with the app token, so it gets the opaque origin.
fn remote_token_origin(origin: Option<String>) -> Option<String> {
    Some(origin.unwrap_or_else(|| navigation::OPAQUE_ORIGIN.to_owned()))
}

/// Answer an app-token request now when the main frame shows app content,
/// otherwise keep it for the next app-content commit (the request can arrive
/// just before the commit of the document that sent it).
fn answer_app_token_request(
    tokens: &IpcTokens,
    label: &str,
    request_id: &str,
    proxy: &EventLoopProxy<UserEvent>,
) {
    if is_app_local_webview(label) {
        send_app_token(proxy, label, request_id, tokens.app());
    } else {
        tokens.defer_request(request_id);
    }
}

/// A main-frame document committed: re-mint the app token when it is remote,
/// answer the deferred requests when it is app content.
fn on_document_committed(tokens: &IpcTokens, label: &str, proxy: &EventLoopProxy<UserEvent>) {
    if !is_app_local_webview(label) {
        tokens.rotate_app();
        return;
    }
    let (pending, token) = tokens.take_pending();
    for request_id in pending {
        send_app_token(proxy, label, &request_id, token.clone());
    }
}

/// Evaluate the answer in the main frame of `label` (location-guarded by
/// [`shim::eval_os_result`]).
fn send_app_token(proxy: &EventLoopProxy<UserEvent>, label: &str, request_id: &str, token: String) {
    let _ = proxy.send_event(UserEvent::OsResult {
        label: label.to_owned(),
        id: request_id.to_owned(),
        ok: true,
        value: Value::String(token),
    });
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
