//! Routes every message coming in over the WebView IPC channel.
//!
//! The WebView handler runs on a wry-owned thread — we keep that closure
//! minimal and delegate the parsing + fan-out here.

use dashmap::DashMap;
use serde_json::Value;
use std::sync::mpsc;
use std::sync::OnceLock;
use tao::event_loop::EventLoopProxy;

use super::util::take_string;
use super::UserEvent;
use crate::{os, runtime::BackendCall};

/// Maps backend call `id` to the window `label` that issued it. Needed so the
/// main loop can route `BackendEvent::Yield` / `Return` back to the webview
/// that originated the streaming call (otherwise secondary windows never see
/// their own yields). Entries are cleared on `Return`.
static CALL_LABELS: OnceLock<DashMap<String, String>> = OnceLock::new();

pub(super) fn call_labels() -> &'static DashMap<String, String> {
    CALL_LABELS.get_or_init(DashMap::default)
}

/// Zero-allocation scan for `"id":"<value>"` in the IPC fast path. The shim
/// produces compact `JSON.stringify` output with deterministic key order, so
/// the substring search is reliable without parsing the whole body.
fn extract_call_id(body: &str) -> Option<String> {
    let after = body.split_once(r#""id":""#)?.1;
    let end = after.find('"')?;
    Some(after[..end].to_string())
}

/// Dispatch one IPC message body. `body` is consumed; the fast path
/// (`{"type":"call",...}`) moves it straight into the backend channel with
/// zero extra allocation. `label` identifies which window sent the message —
/// used so os_call responses can be routed back to the calling webview.
pub(super) fn handle_ipc_body(
    body: String,
    label: &str,
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
        if let Some(id) = extract_call_id(&body) {
            call_labels().insert(id, label.to_string());
        }
        let _ = call_tx.send(BackendCall::Raw(body));
        return;
    }

    let Ok(mut v) = serde_json::from_str::<Value>(&body) else {
        return;
    };

    if let Some("cancel") = v.get("type").and_then(|t| t.as_str()) {
        let id = take_string(&mut v, "id");
        if !id.is_empty() {
            // Drop the label mapping first — otherwise cancelled streams that
            // the backend honours (no Return fired) leak an entry per call.
            call_labels().remove(&id);
            let _ = call_tx.send(BackendCall::Cancel { id });
        }
        return;
    }

    // Credit replenishment from the frontend: the iterator has consumed `n`
    // chunks, so the backend can keep yielding without growing memory. We
    // just forward — backend maintains the per-stream counter.
    if let Some("ack") = v.get("type").and_then(|t| t.as_str()) {
        let id = take_string(&mut v, "id");
        let n = v
            .get("n")
            .and_then(Value::as_u64)
            .map_or(0, |x| x.min(u64::from(u32::MAX)) as u32);
        if !id.is_empty() && n > 0 {
            let _ = call_tx.send(BackendCall::Ack { id, n });
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
            let _ = proxy.send_event(UserEvent::WindowCmd {
                label: label.into(),
                id,
                method,
                args,
            });
        } else {
            // Bounded pool absorbs bursts; overflow falls back to a
            // one-shot thread so urgent calls don't queue behind
            // long-running dialogs (see os::call_pool).
            let proxy = proxy.clone();
            let label_owned = label.to_string();
            os::call_pool::submit(move || {
                let result = os::dispatch(&api, &method, &args);
                let (ok, value) = match result {
                    Ok(v) => (true, v),
                    Err(e) => (false, Value::String(e)),
                };
                let _ = proxy.send_event(UserEvent::OsResult {
                    label: label_owned,
                    id,
                    ok,
                    value,
                });
            });
        }
    }
}
