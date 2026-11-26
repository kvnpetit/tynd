//! Method dispatch table for the debug JSON-RPC protocol.

pub(crate) mod app;
pub(crate) mod capability;
pub(crate) mod ipc;
pub(crate) mod logs;
pub(crate) mod plugin;
pub(crate) mod runtime;
pub(crate) mod screenshot;
pub(crate) mod webview;
pub(crate) mod window;

use serde_json::Value;
use tynd_debug_protocol::{protocol::RpcError, CallOutcome};

/// Dispatch a request to the matching method handler.
///
/// Subscribe methods return a [`CallOutcome::Subscription`] carrying a
/// receiver — the server pumps it on a background thread and pushes
/// notification frames until the client unsubscribes or disconnects.
pub(crate) fn dispatch(method: &str, params: Value) -> CallOutcome {
    match method {
        // App.
        "app.info" => app::info(params),
        "app.quit" => app::quit(params),
        "app.restart" => app::restart(params),
        "app.logs.tail" => logs::tail(params),

        // Window.
        "window.list" => window::list(params),
        "window.info" => window::info(params),
        "window.manage" => window::manage(params),

        // Screenshot.
        "screenshot.window" => screenshot::window(params),
        "screenshot.thumbnail" => screenshot::thumbnail(params),

        // Webview.
        "webview.eval" | "webview.evaluate" => webview::eval(params),
        "webview.snapshot" => webview::snapshot(params),
        "webview.click" => webview::click(params),
        "webview.type" => webview::type_text(params),
        "webview.fill" => webview::fill(params),
        "webview.press_key" => webview::press_key(params),
        "webview.find_element" => webview::find_element(params),
        "webview.scroll" => webview::scroll(params),
        "webview.hover" => webview::hover(params),
        "webview.drag" => webview::drag(params),
        "webview.console_logs" => webview::console_logs(params),
        "webview.network_requests" => webview::network_requests(params),

        // IPC.
        "ipc.subscribe" | "ipc.tap_start" => ipc::subscribe(params),
        "ipc.recent" => ipc::recent(params),
        "ipc.failed" => ipc::failed(params),
        "ipc.replay" => ipc::replay(params),

        // Logs.
        "logs.subscribe" => logs::subscribe(params),

        // Plugin.
        "plugin.list" => plugin::list(params),
        "plugin.docs" => plugin::docs(params),
        "plugin.command_catalog" => plugin::command_catalog(params),

        // Capability.
        "capability.list" => capability::list(params),
        "capability.check" => capability::check(params),
        "capability.suggest_grant" => capability::suggest_grant(params),

        // Runtime.
        "runtime.snapshot" => runtime::snapshot(params),

        other => CallOutcome::Error(RpcError::method_not_found(other)),
    }
}
