//! Native bridge for the WHATWG `WebSocket` global. Delegates the heavy
//! lifting (tungstenite setup, non-blocking read loop, send/close
//! multiplexing, tls stream detection) to
//! `tynd_host::os::websocket::run_session_with` so there's a single
//! implementation of the protocol handling in the workspace.
//!
//! Events are normalised to the polyfill's `ws:*` shape (with the JS-side
//! session id merged in) and delivered via `JsMsg::PolyfillEvent`.

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use rquickjs::{Ctx, Function, Object};
use serde_json::{json, Value};
use std::sync::mpsc;
use std::sync::OnceLock;
use tynd_host::os::websocket::{run_session_with, Cmd};

use super::_session::Sessions;
use crate::quickjs::JsMsg;

/// Process-global session table. The polyfill bridges all hand work
/// off to background threads keyed by JS-side session id, so a single
/// `Sessions<Cmd>` (defined in `_session.rs`) replaces what used to
/// be a `Mutex<HashMap<String, mpsc::Sender<Cmd>>>` declared inline.
fn sessions() -> &'static Sessions<Cmd> {
    static REG: OnceLock<Sessions<Cmd>> = OnceLock::new();
    REG.get_or_init(Sessions::new)
}

pub(crate) fn register<'js>(
    ctx: &Ctx<'js>,
    g: &Object<'js>,
    js_tx: &mpsc::Sender<JsMsg>,
) -> rquickjs::Result<()> {
    {
        let tx = js_tx.clone();
        g.set(
            "__tynd_ws_connect__",
            Function::new(
                ctx.clone(),
                move |id: String, url: String, _protocols_json: String| {
                    let (cmd_tx, cmd_rx) = mpsc::channel::<Cmd>();
                    sessions().insert(id.clone(), cmd_tx);
                    let tx = tx.clone();
                    std::thread::spawn(move || {
                        run_session_with(&url, &cmd_rx, |name, data| {
                            deliver(&tx, &id, name, &data);
                        });
                        sessions().remove(&id);
                    });
                },
            )?,
        )?;
    }

    g.set(
        "__tynd_ws_send__",
        Function::new(
            ctx.clone(),
            |id: String, payload: String, is_binary: bool| {
                let cmd = if is_binary {
                    let bytes = STANDARD.decode(payload.as_bytes()).unwrap_or_default();
                    Cmd::SendBinary(bytes)
                } else {
                    Cmd::SendText(payload)
                };
                sessions().send(&id, cmd);
            },
        )?,
    )?;

    g.set(
        "__tynd_ws_close__",
        Function::new(ctx.clone(), |id: String, code: u16, reason: String| {
            sessions().send(&id, Cmd::Close(Some((code, reason))));
        })?,
    )?;

    Ok(())
}

/// Translate the shared-runner event names ("websocket:*") into the
/// polyfill-side shape ("ws:*") expected by `websocket.js`, and shape the
/// payloads the JS handler needs (merging the id, remapping message kind).
fn deliver(js_tx: &mpsc::Sender<JsMsg>, id: &str, name: &'static str, data: &Value) {
    let (polyfill_name, payload) = match name {
        "websocket:open" => ("ws:open", json!({ "id": id, "protocol": "" })),
        "websocket:message" => {
            let kind = data.get("kind").and_then(Value::as_str).unwrap_or("text");
            let raw = data.get("data").and_then(Value::as_str).unwrap_or("");
            (
                "ws:message",
                json!({
                    "id": id,
                    "payload": raw,
                    "isBinary": kind == "binary",
                }),
            )
        },
        "websocket:error" => (
            "ws:error",
            json!({
                "id": id,
                "message": data.get("message").and_then(Value::as_str).unwrap_or(""),
            }),
        ),
        "websocket:close" => {
            let code = data.get("code").and_then(Value::as_u64).unwrap_or(1000) as u16;
            // Forward the server's close reason instead of always emitting "" —
            // the WHATWG WebSocket spec exposes it on `event.reason`.
            let reason = data.get("reason").and_then(Value::as_str).unwrap_or("");
            (
                "ws:close",
                json!({ "id": id, "code": code, "reason": reason, "wasClean": code == 1000 }),
            )
        },
        _ => return,
    };

    let _ = js_tx.send(JsMsg::PolyfillEvent {
        name: polyfill_name.into(),
        data: payload,
    });
}
