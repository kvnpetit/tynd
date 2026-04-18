//! Routes every message coming in over the WebView IPC channel.
//!
//! The WebView handler runs on a wry-owned thread — we keep that closure
//! minimal and delegate the parsing + fan-out here.

use serde_json::Value;
use std::sync::mpsc;
use tao::event_loop::EventLoopProxy;

use super::util::take_string;
use super::UserEvent;
use crate::{os, runtime::BackendCall};

/// Dispatch one IPC message body. `body` is consumed; the fast path
/// (`{"type":"call",...}`) moves it straight into the backend channel with
/// zero extra allocation.
pub(super) fn handle_ipc_body(
    body: String,
    call_tx: &mpsc::Sender<BackendCall>,
    proxy: &EventLoopProxy<UserEvent>,
) {
    if body.trim().is_empty() {
        return;
    }

    // Fast path: the IPC shim always produces {"type":"call",...} via
    // JSON.stringify (compact, no spaces). Detect without a full parse
    // and forward directly to Bun — owned body moves straight into the
    // channel, no extra allocation on the hot RPC path.
    if body.trim_start().starts_with(r#"{"type":"call""#) {
        let _ = call_tx.send(BackendCall::Raw(body));
        return;
    }

    let Ok(mut v) = serde_json::from_str::<Value>(&body) else {
        return;
    };

    if let Some("cancel") = v.get("type").and_then(|t| t.as_str()) {
        let id = take_string(&mut v, "id");
        if !id.is_empty() {
            let _ = call_tx.send(BackendCall::Cancel { id });
        }
        return;
    }

    // Page-ready signal (fired once on DOMContentLoaded)
    if v.get("__tynd_page_ready")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        let _ = proxy.send_event(UserEvent::PageReady);
        return;
    }

    if let Some("os_call") = v.get("type").and_then(|t| t.as_str()) {
        // Steal owned strings / args out of the parsed Value — avoids
        // clone()s on every OS-API call.
        let id = take_string(&mut v, "id");
        let api = take_string(&mut v, "api");
        let method = take_string(&mut v, "method");
        let args = v.get_mut("args").map_or(Value::Null, std::mem::take);

        if api == "window" {
            // Window commands must run on the main thread.
            let _ = proxy.send_event(UserEvent::WindowCmd { id, method, args });
        } else {
            // Bounded pool absorbs bursts; overflow falls back to a
            // one-shot thread so urgent calls don't queue behind
            // long-running dialogs (see os::call_pool).
            let proxy = proxy.clone();
            os::call_pool::submit(move || {
                let result = os::dispatch(&api, &method, &args);
                let (ok, value) = match result {
                    Ok(v) => (true, v),
                    Err(e) => (false, Value::String(e)),
                };
                let _ = proxy.send_event(UserEvent::OsResult { id, ok, value });
            });
        }
    }
}
