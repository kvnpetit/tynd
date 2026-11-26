//! WebSocket client. Each `connect` spawns a thread that polls the socket
//! in non-blocking mode, emits `websocket:open|message|close|error` events,
//! and drains an outbound `crossbeam_channel` queue. Blocking `tungstenite`
//! under the hood.

use std::{
    io::ErrorKind,
    net::{TcpStream, ToSocketAddrs},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, LazyLock,
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

use base64::{engine::general_purpose::STANDARD, Engine};
use crossbeam_channel::{Receiver, Sender};
use serde_json::{json, Value};
use tungstenite::{
    client::IntoClientRequest,
    handshake::HandshakeError,
    protocol::{CloseFrame, WebSocketConfig},
    stream::MaybeTlsStream,
    Message,
};

use crate::ipc::error::IpcError;

use crate::ipc::owner::Owner;

use parking_lot::{Mutex, RwLock};

use super::registry::Registry;
use crate::api::HostApi;
use crate::tynd;

/// Build the `websocket.<method>: <msg>` IPC error string.
#[cold]
fn ws_err(method: &str, msg: impl std::fmt::Display) -> String {
    IpcError::new("websocket", method, msg).into_string()
}

/// Commands posted to a running WebSocket session.
#[derive(Debug)]
pub(crate) enum Cmd {
    SendText(String),
    SendBinary(Vec<u8>),
    Ping(Vec<u8>),
    Close(Option<(u16, String)>),
}

/// Shared id allocator + session map. `LazyLock` builds the underlying
/// `DashMap` on first reach (DashMap::new is non-const).
#[derive(Clone)]
struct Session {
    tx: Sender<Cmd>,
    stopping: Arc<AtomicBool>,
    join: Arc<Mutex<Option<JoinHandle<()>>>>,
    /// Webview identity that opened the socket; its events go to that webview
    /// only, and only while the label is still on the document generation that
    /// opened it. `None` for a backend-opened socket (default event scope).
    owner: Option<Owner>,
}

static SESSIONS: LazyLock<Registry<Session>> = LazyLock::new(Registry::new);
static SHUTTING_DOWN: RwLock<bool> = RwLock::new(false);

/// Hard cap on a single inbound WebSocket message / frame. Default
/// `tungstenite` allows 64 MiB messages and 16 MiB frames — generous
/// enough that a hostile peer could pin host memory per session.
/// 8 MiB is plenty for the IPC payloads we ferry.
const MAX_WS_MESSAGE_BYTES: usize = 8 * 1024 * 1024;
const WS_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const WS_IO_POLL: Duration = Duration::from_millis(25);
/// How long a locally initiated close waits for the peer's Close frame before
/// the session is torn down anyway.
const CLOSE_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);

type WebSocketConnection = (
    tungstenite::WebSocket<MaybeTlsStream<TcpStream>>,
    tungstenite::handshake::client::Response,
);

#[derive(Debug)]
enum ConnectAttemptError {
    Redirect {
        status: u16,
        location: Option<String>,
    },
    Failure(String),
}

/// Dispatch marker — session state lives in `SESSIONS`.
#[derive(Debug, Default)]
pub(crate) struct WebsocketApi;

/// Row of the host API table (`api::HOST_APIS`).
pub(crate) const API: HostApi = HostApi {
    shutdown: Some(shutdown_all),
    release_webview: Some(release_webview),
    ..HostApi::new("websocket", |method, args| dispatch(method, &args))
};

#[tynd::handler(api = "websocket")]
impl WebsocketApi {
    /// Open a WebSocket connection to `url`. Returns `{ id }` —
    /// pass it back to `send` / `close` / matched-id event filtering.
    pub fn connect(url: String) -> Result<Value, String> {
        let admission = SHUTTING_DOWN.read();
        if *admission {
            return Err(ws_err("connect", "host is shutting down"));
        }
        crate::security::policy::check_http(&url)?;
        // Bounded outbound queue. A fast frontend that calls `ws.send(big)`
        // in a tight loop would otherwise grow the channel without limit
        // while the session thread is mid-write. 256 slots is enough head-
        // room for normal bursts; past that, `send_impl` surfaces a
        // queue-full error to the caller.
        let (tx, rx) = crossbeam_channel::bounded::<Cmd>(256);
        let session = Session {
            tx,
            stopping: Arc::new(AtomicBool::new(false)),
            join: Arc::new(Mutex::new(None)),
            owner: crate::ipc::owner::current(),
        };
        let owner = session.owner.clone();
        let creator_gone = owner.as_ref().is_some_and(|owner| !owner.is_current());
        if creator_gone {
            return Err(gone_error());
        }
        let id = SESSIONS.insert(session.clone());
        let stopping = Arc::clone(&session.stopping);

        let join = std::thread::Builder::new()
            .name(format!("tynd-websocket-{id}"))
            .spawn(move || run_session(id, owner.as_ref(), &url, &rx, &stopping))
            .map_err(|error| {
                SESSIONS.remove(id);
                ws_err("connect", format!("thread spawn failed: {error}"))
            })?;
        *session.join.lock() = Some(join);
        drop(admission);
        // The webview may have committed a new document while the session was
        // being set up. A release that ran before the insert did not see the
        // socket, so close it here (the bump precedes the release scan, so a
        // socket that passes this check is found by that scan). Its events are
        // dropped for the old generation either way.
        if session
            .owner
            .as_ref()
            .is_some_and(|owner| !owner.is_current())
        {
            close_for_release(&session);
            return Err(gone_error());
        }
        Ok(json!({ "id": id }))
    }

    /// Send a frame on the session identified by `id`. `kind` is
    /// `"text"` (default), `"binary"` (`data` is base64), or
    /// `"ping"` (no payload). Errors when the session has gone away.
    pub fn send(id: u64, kind: Option<String>, data: Option<String>) -> Result<(), String> {
        send_impl(id, kind.as_deref().unwrap_or("text"), data.as_deref())
    }

    /// Initiate a graceful close on the session identified by `id`.
    /// `code` and `reason` are forwarded as the WebSocket close
    /// frame; both default to "no reason".
    pub fn close(id: u64, code: Option<u64>, reason: Option<String>) -> Result<(), String> {
        close_impl(id, code.map(|c| c as u16), reason)
    }

    /// List every session id currently held by the host.
    pub fn list() -> Vec<Value> {
        SESSIONS
            .ids()
            .into_iter()
            .filter(|id| {
                SESSIONS
                    .get_cloned(*id)
                    .is_some_and(|session| crate::ipc::owner::permits(session.owner.as_ref()))
            })
            .map(|id| Value::Number(id.into()))
            .collect()
    }
}

fn run_session(
    id: u64,
    owner: Option<&Owner>,
    url: &str,
    rx: &Receiver<Cmd>,
    stopping: &AtomicBool,
) {
    run_session_with_control(url, rx, Some(stopping), |name, mut data| {
        // Merge the session id into the shared payload shape.
        if let Some(obj) = data.as_object_mut() {
            obj.insert("id".into(), Value::Number(id.into()));
        }
        crate::ipc::owner::emit_for(owner, name, &data);
    });
    SESSIONS.remove(id);
}

fn run_session_with_control<F>(
    url: &str,
    rx: &Receiver<Cmd>,
    stopping: Option<&AtomicBool>,
    mut sink: F,
) where
    F: FnMut(&'static str, Value),
{
    let cfg = websocket_config();
    let (ws, _resp) = match connect_with_timeout(url, cfg, WS_CONNECT_TIMEOUT) {
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

    drive_session(ws, rx, stopping, &mut sink, CLOSE_HANDSHAKE_TIMEOUT);
}

/// Pump an established session until it ends: drain outbound commands, read
/// inbound frames, and leave as soon as the session is over (peer close, I/O
/// error, failed send) or a locally initiated close was not answered within
/// `close_timeout`.
fn drive_session<F>(
    mut ws: tungstenite::WebSocket<MaybeTlsStream<TcpStream>>,
    rx: &Receiver<Cmd>,
    stopping: Option<&AtomicBool>,
    sink: &mut F,
    close_timeout: Duration,
) where
    F: FnMut(&'static str, Value),
{
    // Blocking read with a short timeout parks the thread between frames
    // while keeping outbound send latency bounded (~25 ms).
    let read_timeout = Some(WS_IO_POLL);
    match ws.get_ref() {
        MaybeTlsStream::Plain(s) => {
            let _ = s.set_read_timeout(read_timeout);
        },
        MaybeTlsStream::Rustls(s) => {
            let _ = s.get_ref().set_read_timeout(read_timeout);
        },
        _ => {},
    }

    sink("websocket:open", json!({}));

    // Set once we sent our Close frame: the peer must answer before it.
    let mut close_deadline: Option<Instant> = None;

    loop {
        if stopping.is_some_and(|flag| flag.load(Ordering::Acquire)) {
            let _ = ws.close(None);
            break;
        }
        if close_deadline.is_some_and(|deadline| Instant::now() >= deadline) {
            sink(
                "websocket:error",
                json!({ "message": "close handshake timed out" }),
            );
            sink("websocket:close", json!({ "code": 1006 }));
            break;
        }
        if close_deadline.is_some() {
            // Nothing may follow our Close frame; drop late commands.
            while rx.try_recv().is_ok() {}
        } else if let Err(e) = drain_commands(&mut ws, rx, &mut close_deadline, close_timeout) {
            sink("websocket:error", json!({ "message": e.to_string() }));
            sink("websocket:close", json!({ "code": 1006 }));
            break;
        }

        if read_inbound(&mut ws, sink) {
            break;
        }
    }
}

/// Send every queued outbound command. A `Close` command sends our Close
/// frame, arms `close_deadline` and stops draining: nothing may follow it.
fn drain_commands(
    ws: &mut tungstenite::WebSocket<MaybeTlsStream<TcpStream>>,
    rx: &Receiver<Cmd>,
    close_deadline: &mut Option<Instant>,
    close_timeout: Duration,
) -> tungstenite::Result<()> {
    while let Ok(cmd) = rx.try_recv() {
        match cmd {
            Cmd::SendText(s) => ws.send(Message::Text(s.into()))?,
            Cmd::SendBinary(b) => ws.send(Message::Binary(b.into()))?,
            Cmd::Ping(b) => ws.send(Message::Ping(b.into()))?,
            Cmd::Close(reason) => {
                let frame = reason.map(|(code, r)| CloseFrame {
                    code: tungstenite::protocol::frame::coding::CloseCode::from(code),
                    reason: r.into(),
                });
                let _ = ws.close(frame);
                close_deadline.get_or_insert_with(|| Instant::now() + close_timeout);
                // Keep reading so the peer's Close ends the session, but
                // send nothing after our Close frame.
                return Ok(());
            },
        }
    }
    Ok(())
}

/// Read at most one inbound frame (bounded by the socket read timeout) and
/// forward it to `sink`. Returns `true` once the session is over.
fn read_inbound<F>(ws: &mut tungstenite::WebSocket<MaybeTlsStream<TcpStream>>, sink: &mut F) -> bool
where
    F: FnMut(&'static str, Value),
{
    match ws.read() {
        Ok(Message::Text(s)) => {
            sink(
                "websocket:message",
                json!({ "kind": "text", "data": s.as_str() }),
            );
            false
        },
        Ok(Message::Binary(b)) => {
            sink(
                "websocket:message",
                json!({ "kind": "binary", "data": STANDARD.encode(&b) }),
            );
            false
        },
        Ok(Message::Ping(_) | Message::Pong(_) | Message::Frame(_)) => false,
        Ok(Message::Close(frame)) => {
            let code = frame.as_ref().map_or(1000, |f| u16::from(f.code));
            sink("websocket:close", json!({ "code": code }));
            true
        },
        Err(tungstenite::Error::Io(e))
            if e.kind() == ErrorKind::WouldBlock || e.kind() == ErrorKind::TimedOut =>
        {
            false
        },
        Err(tungstenite::Error::ConnectionClosed | tungstenite::Error::AlreadyClosed) => {
            sink("websocket:close", json!({ "code": 1000 }));
            true
        },
        Err(e) => {
            sink("websocket:error", json!({ "message": e.to_string() }));
            true
        },
    }
}

fn websocket_config() -> WebSocketConfig {
    // Bound message + frame size explicitly. Default tungstenite caps
    // are 64 MiB / 16 MiB respectively; tighten to keep a hostile or
    // chatty peer from inflating per-session memory.
    // `max_write_buffer_size` is also bounded (8 MiB). Past that cap a
    // fast `ws.send(big)` blocks (or surfaces an error from
    // `tungstenite`) instead of letting the per-session write buffer
    // grow without limit while the network drains slowly.
    WebSocketConfig::default()
        .write_buffer_size(128 * 1024)
        .max_write_buffer_size(8 * 1024 * 1024)
        .max_message_size(Some(MAX_WS_MESSAGE_BYTES))
        .max_frame_size(Some(MAX_WS_MESSAGE_BYTES))
        .accept_unmasked_frames(false)
}

fn connect_with_timeout(
    url: &str,
    config: WebSocketConfig,
    timeout: Duration,
) -> Result<WebSocketConnection, String> {
    const MAX_REDIRECTS: usize = 3;
    let deadline = Instant::now() + timeout;
    let mut current =
        url::Url::parse(url).map_err(|error| format!("invalid WebSocket URL: {error}"))?;

    for attempt in 0..=MAX_REDIRECTS {
        crate::security::policy::check_http(current.as_str())
            .map_err(|error| format!("WebSocket URL denied by HTTP policy: {error}"))?;
        let request = current
            .as_str()
            .into_client_request()
            .map_err(|error| error.to_string())?;
        match connect_request_with_deadline(request, config, deadline) {
            Err(ConnectAttemptError::Redirect { status, location }) => {
                if attempt == MAX_REDIRECTS {
                    return Err(format!(
                        "WebSocket redirect limit exceeded after HTTP {status}"
                    ));
                }
                let Some(location) = location else {
                    return Err(format!(
                        "WebSocket redirect response HTTP {status} has no valid Location header"
                    ));
                };
                current = current
                    .join(&location)
                    .map_err(|error| format!("invalid WebSocket redirect URL: {error}"))?;
            },
            Err(ConnectAttemptError::Failure(error)) => return Err(error),
            Ok(connection) => return Ok(connection),
        }
    }
    unreachable!("redirect loop returns on its final attempt")
}

fn connect_request_with_deadline(
    request: tungstenite::handshake::client::Request,
    config: WebSocketConfig,
    deadline: Instant,
) -> Result<WebSocketConnection, ConnectAttemptError> {
    let uri = request.uri();
    let host = uri
        .host()
        .ok_or_else(|| ConnectAttemptError::Failure("WebSocket URL has no host".to_owned()))?;
    let host = host
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        .unwrap_or(host);
    let port = uri.port_u16().unwrap_or_else(|| {
        if uri.scheme_str() == Some("wss") {
            443
        } else {
            80
        }
    });
    let stream = connect_tcp(host, port, deadline)?;
    stream
        .set_nodelay(true)
        .map_err(|error| ConnectAttemptError::Failure(error.to_string()))?;
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return Err(ConnectAttemptError::Failure(
            "websocket handshake deadline elapsed".to_owned(),
        ));
    }
    stream
        .set_read_timeout(Some(remaining))
        .map_err(|error| ConnectAttemptError::Failure(error.to_string()))?;
    stream
        .set_write_timeout(Some(remaining))
        .map_err(|error| ConnectAttemptError::Failure(error.to_string()))?;

    tungstenite::client_tls_with_config(request, stream, Some(config), None)
        .map_err(handshake_error)
}

/// Connect to the first resolved address of `host:port` that answers before
/// `deadline`.
fn connect_tcp(host: &str, port: u16, deadline: Instant) -> Result<TcpStream, ConnectAttemptError> {
    let addresses = (host, port)
        .to_socket_addrs()
        .map_err(|error| ConnectAttemptError::Failure(error.to_string()))?;
    let mut last_error = None;
    for address in addresses {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        match TcpStream::connect_timeout(&address, remaining) {
            Ok(stream) => return Ok(stream),
            Err(error) => last_error = Some(error),
        }
    }
    Err(ConnectAttemptError::Failure(last_error.map_or_else(
        || "websocket connection deadline elapsed".to_owned(),
        |error: std::io::Error| error.to_string(),
    )))
}

/// Classify a failed handshake: an HTTP redirect is surfaced separately so
/// the caller can re-check the target against the HTTP policy.
fn handshake_error<S>(error: HandshakeError<S>) -> ConnectAttemptError
where
    S: tungstenite::handshake::HandshakeRole,
{
    match error {
        HandshakeError::Failure(tungstenite::Error::Http(response))
            if response.status().is_redirection() =>
        {
            ConnectAttemptError::Redirect {
                status: response.status().as_u16(),
                location: response
                    .headers()
                    .get("location")
                    .and_then(|value| value.to_str().ok())
                    .map(str::to_owned),
            }
        },
        HandshakeError::Failure(error) => ConnectAttemptError::Failure(error.to_string()),
        HandshakeError::Interrupted(_) => {
            ConnectAttemptError::Failure("websocket handshake interrupted".to_owned())
        },
    }
}

fn session_for(id: u64) -> Result<Session, String> {
    // A socket owned by another webview answers like an unknown id.
    SESSIONS
        .get_cloned(id)
        .filter(|session| crate::ipc::owner::permits(session.owner.as_ref()))
        .ok_or_else(|| ws_err("*", format!("session {id} not found")))
}

fn send_impl(id: u64, kind: &str, data: Option<&str>) -> Result<(), String> {
    let session = session_for(id)?;
    // SECURITY: outbound payload MUST be capped against
    // `MAX_WS_MESSAGE_BYTES` BEFORE base64 decoding. A caller can
    // otherwise hand us a 4 GiB base64 blob and pin a multi-GiB `Vec<u8>`
    // pre-encode while we silently allocate. Same logic as
    // `fs/handles.rs` does on its base64 inputs. The bound is `b64_len <=
    // ceil(MAX * 4 / 3)` since base64 expands 3→4 (plus padding).
    let max_b64_len = MAX_WS_MESSAGE_BYTES.saturating_mul(4) / 3 + 4;
    let cmd = match kind {
        "text" => {
            let s = data.ok_or_else(|| ws_err("send", "missing text 'data'"))?;
            if s.len() > MAX_WS_MESSAGE_BYTES {
                return Err(ws_err(
                    "send",
                    format!(
                        "text payload {} bytes exceeds cap {MAX_WS_MESSAGE_BYTES}",
                        s.len()
                    ),
                ));
            }
            Cmd::SendText(s.to_string())
        },
        "binary" => {
            let b64 = data.ok_or_else(|| ws_err("send", "missing base64 'data'"))?;
            if b64.len() > max_b64_len {
                return Err(ws_err(
                    "send",
                    format!(
                        "binary payload (base64 {} bytes) exceeds cap {MAX_WS_MESSAGE_BYTES}",
                        b64.len()
                    ),
                ));
            }
            Cmd::SendBinary(
                STANDARD
                    .decode(b64)
                    .map_err(|e| ws_err("send", format!("invalid base64: {e}")))?,
            )
        },
        "ping" => Cmd::Ping(Vec::new()),
        other => return Err(ws_err("send", format!("unknown kind '{other}'"))),
    };
    session.tx.try_send(cmd).map_err(|error| match error {
        crossbeam_channel::TrySendError::Full(_) => ws_err("send", "outbound queue is full"),
        crossbeam_channel::TrySendError::Disconnected(_) => ws_err("send", "session gone"),
    })?;
    Ok(())
}

fn close_impl(id: u64, code: Option<u16>, reason: Option<String>) -> Result<(), String> {
    let session = session_for(id)?;
    let cmd = match (code, reason) {
        (Some(c), Some(r)) => Cmd::Close(Some((c, r))),
        (Some(c), None) => Cmd::Close(Some((c, String::new()))),
        _ => Cmd::Close(None),
    };
    match session.tx.try_send(cmd) {
        Ok(()) => Ok(()),
        Err(crossbeam_channel::TrySendError::Full(_)) => {
            // A saturated data queue must not make close impossible. The
            // session loop checks this flag before its next 25 ms read poll.
            session.stopping.store(true, Ordering::Release);
            Ok(())
        },
        Err(crossbeam_channel::TrySendError::Disconnected(_)) => {
            Err(ws_err("close", "session gone"))
        },
    }
}

fn close_for_release(session: &Session) {
    session.stopping.store(true, Ordering::Release);
    let _ = session.tx.try_send(Cmd::Close(None));
}

fn gone_error() -> String {
    ws_err("connect", "the document that requested it is gone")
}

/// Close the sockets `label` opened. The session threads finish on their own
/// and unregister themselves; nothing here blocks on them.
pub(crate) fn release_webview(label: &str) {
    for id in SESSIONS.ids() {
        let Some(session) = SESSIONS.get_cloned(id) else {
            continue;
        };
        if session
            .owner
            .as_ref()
            .is_some_and(|owner| owner.label() == label)
        {
            close_for_release(&session);
        }
    }
}

pub(crate) fn shutdown_all() {
    let mut shutting_down = SHUTTING_DOWN.write();
    *shutting_down = true;
    let sessions = SESSIONS
        .ids()
        .into_iter()
        .filter_map(|id| SESSIONS.get_cloned(id).map(|session| (id, session)))
        .collect::<Vec<_>>();

    drop(shutting_down);

    shutdown_sessions(sessions);
}

fn shutdown_sessions(sessions: Vec<(u64, Session)>) {
    for (_, session) in &sessions {
        session.stopping.store(true, Ordering::Release);
        let _ = session.tx.try_send(Cmd::Close(None));
    }
    for (id, session) in sessions {
        if let Some(join) = session.join.lock().take() {
            if join.join().is_err() {
                tracing::warn!(
                    target: "tynd::cleanup",
                    id,
                    "websocket session thread panicked during shutdown"
                );
            }
        }
        SESSIONS.remove(id);
    }
}

#[cfg(test)]
mod tests;
