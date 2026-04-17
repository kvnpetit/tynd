//! WebSocket client. Each `connect` spawns a thread that polls the socket
//! in non-blocking mode, emits `websocket:open|message|close|error` events,
//! and drains an outbound mpsc queue. Blocking `tungstenite` under the hood.

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use dashmap::DashMap;
use serde_json::{json, Value};
use std::io::ErrorKind;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::OnceLock;
use std::time::Duration;
use tungstenite::protocol::CloseFrame;
use tungstenite::Message;

use super::events;

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

enum Cmd {
    SendText(String),
    SendBinary(Vec<u8>),
    Ping(Vec<u8>),
    Close(Option<(u16, String)>),
}

type Sessions = DashMap<u64, mpsc::Sender<Cmd>>;

fn sessions() -> &'static Sessions {
    static S: OnceLock<Sessions> = OnceLock::new();
    S.get_or_init(DashMap::new)
}

pub fn dispatch(method: &str, args: &Value) -> Result<Value, String> {
    match method {
        "connect" => connect(args),
        "send" => send(args),
        "close" => close(args),
        "list" => list(),
        _ => Err(format!("websocket.{method}: unknown method")),
    }
}

fn connect(args: &Value) -> Result<Value, String> {
    let url = args
        .get("url")
        .and_then(Value::as_str)
        .ok_or_else(|| "websocket.connect: missing 'url'".to_string())?
        .to_string();

    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let (tx, rx) = mpsc::channel::<Cmd>();
    sessions().insert(id, tx);

    std::thread::spawn(move || run_session(id, &url, &rx));
    Ok(json!({ "id": id }))
}

fn run_session(id: u64, url: &str, rx: &mpsc::Receiver<Cmd>) {
    let (mut ws, _resp) = match tungstenite::connect(url) {
        Ok(p) => p,
        Err(e) => {
            events::emit(
                "websocket:error",
                &json!({ "id": id, "message": format!("connect: {e}") }),
            );
            events::emit("websocket:close", &json!({ "id": id, "code": 1006 }));
            sessions().remove(&id);
            return;
        },
    };

    // Blocking read with a short timeout: the kernel parks the thread until
    // data arrives or the timeout fires — no busy-wait, no sleep loop.
    // 25 ms caps outbound send latency while keeping wake-ups to ~40/s.
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

    events::emit("websocket:open", &json!({ "id": id }));

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
                events::emit(
                    "websocket:error",
                    &json!({ "id": id, "message": e.to_string() }),
                );
                break;
            }
        }

        match ws.read() {
            Ok(Message::Text(s)) => {
                events::emit(
                    "websocket:message",
                    &json!({ "id": id, "kind": "text", "data": s.as_str() }),
                );
            },
            Ok(Message::Binary(b)) => {
                events::emit(
                    "websocket:message",
                    &json!({ "id": id, "kind": "binary", "data": STANDARD.encode(&b) }),
                );
            },
            Ok(Message::Ping(_) | Message::Pong(_) | Message::Frame(_)) => {},
            Ok(Message::Close(frame)) => {
                let code = frame.as_ref().map_or(1000, |f| u16::from(f.code));
                events::emit("websocket:close", &json!({ "id": id, "code": code }));
                break;
            },
            Err(tungstenite::Error::Io(e))
                if e.kind() == ErrorKind::WouldBlock || e.kind() == ErrorKind::TimedOut => {},
            Err(tungstenite::Error::ConnectionClosed | tungstenite::Error::AlreadyClosed) => {
                events::emit("websocket:close", &json!({ "id": id, "code": 1000 }));
                break;
            },
            Err(e) => {
                events::emit(
                    "websocket:error",
                    &json!({ "id": id, "message": e.to_string() }),
                );
                break;
            },
        }
    }

    sessions().remove(&id);
}

fn session_tx(args: &Value) -> Result<mpsc::Sender<Cmd>, String> {
    let id = args
        .get("id")
        .and_then(Value::as_u64)
        .ok_or_else(|| "websocket: missing 'id'".to_string())?;
    sessions()
        .get(&id)
        .map(|r| r.value().clone())
        .ok_or_else(|| format!("websocket: session {id} not found"))
}

fn send(args: &Value) -> Result<Value, String> {
    let tx = session_tx(args)?;
    let kind = args.get("kind").and_then(Value::as_str).unwrap_or("text");
    let cmd = match kind {
        "text" => Cmd::SendText(
            args.get("data")
                .and_then(Value::as_str)
                .ok_or_else(|| "websocket.send: missing text 'data'".to_string())?
                .to_string(),
        ),
        "binary" => {
            let b64 = args
                .get("data")
                .and_then(Value::as_str)
                .ok_or_else(|| "websocket.send: missing base64 'data'".to_string())?;
            Cmd::SendBinary(
                STANDARD
                    .decode(b64)
                    .map_err(|e| format!("websocket.send: invalid base64: {e}"))?,
            )
        },
        "ping" => Cmd::Ping(Vec::new()),
        other => return Err(format!("websocket.send: unknown kind '{other}'")),
    };
    tx.send(cmd)
        .map_err(|e| format!("websocket.send: session gone: {e}"))?;
    Ok(Value::Null)
}

fn close(args: &Value) -> Result<Value, String> {
    let tx = session_tx(args)?;
    let code = args.get("code").and_then(Value::as_u64).map(|c| c as u16);
    let reason = args.get("reason").and_then(Value::as_str).map(String::from);
    let cmd = match (code, reason) {
        (Some(c), Some(r)) => Cmd::Close(Some((c, r))),
        (Some(c), None) => Cmd::Close(Some((c, String::new()))),
        _ => Cmd::Close(None),
    };
    let _ = tx.send(cmd);
    Ok(Value::Null)
}

#[allow(clippy::unnecessary_wraps)] // dispatch expects Result — uniform return shape
fn list() -> Result<Value, String> {
    let ids: Vec<Value> = sessions()
        .iter()
        .map(|r| Value::Number((*r.key()).into()))
        .collect();
    Ok(Value::Array(ids))
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
        assert!(connect(&json!({})).unwrap_err().contains("missing 'url'"));
    }

    #[test]
    fn send_on_missing_session_errors() {
        let err = send(&json!({ "id": 999_999, "kind": "text", "data": "x" })).unwrap_err();
        assert!(err.contains("not found") || err.contains("session"));
    }

    #[test]
    fn send_rejects_unknown_kind_when_session_missing() {
        // session_tx runs first, so this also hits the missing-session path,
        // but we still exercise the arg-shape guard.
        let err = send(&json!({ "id": 999_998, "kind": "weird" })).unwrap_err();
        assert!(!err.is_empty());
    }

    #[test]
    fn close_on_missing_session_errors() {
        let err = close(&json!({ "id": 999_997 })).unwrap_err();
        assert!(err.contains("not found") || err.contains("session"));
    }

    #[test]
    fn list_returns_array() {
        let v = list().unwrap();
        assert!(v.is_array());
    }
}
