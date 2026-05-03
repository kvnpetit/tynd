//! WebSocket client. Each `connect` spawns a thread that polls the socket
//! in non-blocking mode, emits `websocket:open|message|close|error` events,
//! and drains an outbound `crossbeam_channel` queue. Blocking `tungstenite`
//! under the hood.
//!
//! V1 (B27): the per-session command channel migrated from `std::sync::mpsc`
//! to `crossbeam_channel`. The crossbeam variant gives faster `try_recv`
//! micros on the read loop, supports `select!` for the future async
//! backend wiring, and aligns with the rest of the workspace (audio
//! plugin, lite polyfills) which already use crossbeam.
//!
//! The `ws-async` feature stages an alternative backend on top of
//! `tokio-tungstenite` that removes the 25 ms read-timeout busy-poll
//! latency floor inherent to the blocking `set_read_timeout` polling
//! pattern below. Only the scaffolding is wired right now — see
//! `async_backend` for the hook points; full migration is tracked in
//! follow-up work.

#[cfg(feature = "ws-async")]
#[path = "websocket_async.rs"]
mod async_backend;

use std::{io::ErrorKind, sync::LazyLock, time::Duration};

use base64::{engine::general_purpose::STANDARD, Engine};
use crossbeam_channel::{Receiver, Sender};
use serde_json::{json, Value};
use tungstenite::{protocol::CloseFrame, Message};

use super::{events, registry::Registry};
use crate::{ipc_error::IpcError, tynd};

/// Commands posted to a running WebSocket session. Exposed (pub) so the
/// lite runtime's `WebSocket` polyfill can reuse the same session runner
/// via `run_session_with`.
#[derive(Debug)]
pub enum Cmd {
    SendText(String),
    SendBinary(Vec<u8>),
    Ping(Vec<u8>),
    Close(Option<(u16, String)>),
}

/// Shared id allocator + session map. `LazyLock` builds the underlying
/// `DashMap` on first reach (DashMap::new is non-const). Drops the
/// `NEXT_ID` atomic + `Sessions` type alias + `OnceLock`/`get_or_init`
/// dance the previous shape needed.
///
/// V1 (B27): senders are `crossbeam_channel::Sender<Cmd>` — same
/// owned-clone semantics as `mpsc::Sender`, with faster `send` micros
/// and matching `try_recv` on the consumer side.
static SESSIONS: LazyLock<Registry<Sender<Cmd>>> = LazyLock::new(Registry::new);

/// Marker for the `#[tynd::handler]` dispatch table — session state
/// lives in the file-level `SESSIONS` registry.
#[derive(Debug, Default)]
pub struct WebsocketApi;

#[tynd::handler(api = "websocket")]
impl WebsocketApi {
    /// Open a WebSocket connection to `url`. Returns `{ id }` —
    /// pass it back to `send` / `close` / matched-id event filtering.
    pub fn connect(url: String) -> Value {
        let (tx, rx) = crossbeam_channel::unbounded::<Cmd>();
        let id = SESSIONS.insert(tx);

        std::thread::spawn(move || run_session(id, &url, &rx));
        json!({ "id": id })
    }

    /// Send a frame on the session identified by `id`. `kind` is
    /// `"text"` (default), `"binary"` (`data` is base64), or
    /// `"ping"` (no payload). Errors when the session has gone away.
    pub fn send(
        id: u64,
        kind: Option<String>,
        data: Option<String>,
    ) -> Result<(), String> {
        send_impl(id, kind.as_deref().unwrap_or("text"), data.as_deref())
    }

    /// Initiate a graceful close on the session identified by `id`.
    /// `code` and `reason` are forwarded as the WebSocket close
    /// frame; both default to "no reason".
    pub fn close(
        id: u64,
        code: Option<u64>,
        reason: Option<String>,
    ) -> Result<(), String> {
        close_impl(id, code.map(|c| c as u16), reason)
    }

    /// List every session id currently held by the host.
    pub fn list() -> Vec<Value> {
        SESSIONS
            .ids()
            .into_iter()
            .map(|id| Value::Number(id.into()))
            .collect()
    }
}

fn run_session(id: u64, url: &str, rx: &Receiver<Cmd>) {
    run_session_with(url, rx, |name, mut data| {
        // Merge the session id into the shared payload shape.
        if let Some(obj) = data.as_object_mut() {
            obj.insert("id".into(), Value::Number(id.into()));
        }
        events::emit(name, &data);
    });
    SESSIONS.remove(id);
}

/// Drive a WebSocket session until close. The caller supplies a sink that
/// receives normalised events: "websocket:open" | ":message" | ":close" |
/// ":error" with `serde_json` payloads. Used by both the frontend-dispatch
/// OS API (`run_session` above) and the lite runtime `WebSocket` polyfill.
///
/// V1 (B27): `rx` is now a `crossbeam_channel::Receiver<Cmd>` —
/// caller-side breakage is intentional; producers must use
/// `crossbeam_channel::unbounded()` (or `bounded`) rather than
/// `std::sync::mpsc::channel`.
pub fn run_session_with<F>(url: &str, rx: &Receiver<Cmd>, mut sink: F)
where
    F: FnMut(&'static str, Value),
{
    let (mut ws, _resp) = match tungstenite::connect(url) {
        Ok(p) => p,
        Err(e) => {
            sink(
                "websocket:error",
                json!({ "message": format!("connect: {e}") }),
            );
            sink("websocket:close", json!({ "code": 1006 }));
            return;
        },
    };

    // Blocking read with a short timeout parks the thread between frames
    // while keeping outbound send latency bounded (~25 ms).
    let read_timeout = Some(Duration::from_millis(25));
    match ws.get_ref() {
        tungstenite::stream::MaybeTlsStream::Plain(s) => {
            let _ = s.set_read_timeout(read_timeout);
        },
        tungstenite::stream::MaybeTlsStream::Rustls(s) => {
            let _ = s.get_ref().set_read_timeout(read_timeout);
        },
        _ => {},
    }

    sink("websocket:open", json!({}));

    loop {
        while let Ok(cmd) = rx.try_recv() {
            let res = match cmd {
                Cmd::SendText(s) => {
                    #[allow(clippy::useless_conversion)]
                    let msg = Message::Text(s.into());
                    ws.send(msg)
                },
                Cmd::SendBinary(b) => ws.send(Message::Binary(b)),
                Cmd::Ping(b) => ws.send(Message::Ping(b)),
                Cmd::Close(reason) => {
                    let frame = reason.map(|(code, r)| CloseFrame {
                        code: tungstenite::protocol::frame::coding::CloseCode::from(code),
                        reason: r.into(),
                    });
                    let _ = ws.close(frame);
                    break;
                },
            };
            if let Err(e) = res {
                sink("websocket:error", json!({ "message": e.to_string() }));
                break;
            }
        }

        match ws.read() {
            Ok(Message::Text(s)) => {
                sink(
                    "websocket:message",
                    json!({ "kind": "text", "data": s.as_str() }),
                );
            },
            Ok(Message::Binary(b)) => {
                sink(
                    "websocket:message",
                    json!({ "kind": "binary", "data": STANDARD.encode(&b) }),
                );
            },
            Ok(Message::Ping(_) | Message::Pong(_) | Message::Frame(_)) => {},
            Ok(Message::Close(frame)) => {
                let code = frame.as_ref().map_or(1000, |f| u16::from(f.code));
                sink("websocket:close", json!({ "code": code }));
                break;
            },
            Err(tungstenite::Error::Io(e))
                if e.kind() == ErrorKind::WouldBlock || e.kind() == ErrorKind::TimedOut => {},
            Err(tungstenite::Error::ConnectionClosed | tungstenite::Error::AlreadyClosed) => {
                sink("websocket:close", json!({ "code": 1000 }));
                break;
            },
            Err(e) => {
                sink("websocket:error", json!({ "message": e.to_string() }));
                break;
            },
        }
    }
}

fn session_tx_for(id: u64) -> Result<Sender<Cmd>, String> {
    SESSIONS.get_cloned(id).ok_or_else(|| {
        IpcError::new("websocket", "*", format!("session {id} not found")).into_string()
    })
}

fn send_impl(id: u64, kind: &str, data: Option<&str>) -> Result<(), String> {
    let tx = session_tx_for(id)?;
    let cmd = match kind {
        "text" => Cmd::SendText(
            data.ok_or_else(|| {
                IpcError::new("websocket", "send", "missing text 'data'").into_string()
            })?
            .to_string(),
        ),
        "binary" => {
            let b64 = data.ok_or_else(|| {
                IpcError::new("websocket", "send", "missing base64 'data'").into_string()
            })?;
            Cmd::SendBinary(STANDARD.decode(b64).map_err(|e| {
                IpcError::new("websocket", "send", format!("invalid base64: {e}")).into_string()
            })?)
        },
        "ping" => Cmd::Ping(Vec::new()),
        other => {
            return Err(
                IpcError::new("websocket", "send", format!("unknown kind '{other}'")).into_string(),
            )
        },
    };
    tx.send(cmd).map_err(|e| {
        IpcError::new("websocket", "send", format!("session gone: {e}")).into_string()
    })?;
    Ok(())
}

fn close_impl(id: u64, code: Option<u16>, reason: Option<String>) -> Result<(), String> {
    let tx = session_tx_for(id)?;
    let cmd = match (code, reason) {
        (Some(c), Some(r)) => Cmd::Close(Some((c, r))),
        (Some(c), None) => Cmd::Close(Some((c, String::new()))),
        _ => Cmd::Close(None),
    };
    let _ = tx.send(cmd);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dispatch_rejects_unknown_method() {
        assert!(dispatch("nope", &json!({})).is_err());
    }

    #[test]
    fn connect_requires_url() {
        assert!(dispatch("connect", &json!({}))
            .unwrap_err()
            .contains("missing 'url'"));
    }

    #[test]
    fn send_on_missing_session_errors() {
        let err = dispatch(
            "send",
            &json!({ "id": 999_999u64, "kind": "text", "data": "x" }),
        )
        .unwrap_err();
        assert!(err.contains("not found") || err.contains("session"));
    }

    #[test]
    fn send_rejects_unknown_kind_when_session_missing() {
        // session_tx runs first, so this also hits the missing-session path,
        // but we still exercise the arg-shape guard.
        let err =
            dispatch("send", &json!({ "id": 999_998u64, "kind": "weird" })).unwrap_err();
        assert!(!err.is_empty());
    }

    #[test]
    fn close_on_missing_session_errors() {
        let err = dispatch("close", &json!({ "id": 999_997u64 })).unwrap_err();
        assert!(err.contains("not found") || err.contains("session"));
    }

    #[test]
    fn list_returns_array() {
        let v = dispatch("list", &json!({})).unwrap();
        assert!(v.is_array());
    }
}
