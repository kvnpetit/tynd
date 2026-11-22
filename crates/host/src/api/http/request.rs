use std::{
    fmt::Display,
    io::Read,
    mem,
    sync::{atomic::AtomicBool, mpsc, Arc},
    time::{Duration, Instant},
};

use base64::{engine::general_purpose::STANDARD, Engine};
use bytes::{Bytes, BytesMut};
use serde_json::{json, Value};

use crate::ipc::error::IpcError;

use super::{
    check_url_scheme, header_str, string_headers,
    transport::{execute_watched, AbortHandle, AbortRead, Exchange, Net, Outgoing},
    ProgressEmitter, RequestArgs, DEFAULT_CONNECT_TIMEOUT, DEFAULT_STREAM_IDLE_TIMEOUT,
    DEFAULT_TIMEOUT, MAX_REQUEST_BODY, MAX_RESPONSE_BODY,
};

type Response = ureq::http::Response<ureq::Body>;

/// `http.<op>: <msg>` error string.
fn op_error(op: &str, msg: impl Display) -> String {
    IpcError::new("http", op, msg).into_string()
}

/// A validated call ready to run: everything `request` and `stream` share.
struct Prepared {
    agent: ureq::Agent,
    outgoing: Outgoing,
    net: Net,
    abort: AbortHandle,
}

/// `default_timeout` applies without `timeoutMs`; a call left with no
/// overall deadline still gets a connect deadline. `idle` caps each
/// transport read (see `Net::agent`).
fn prepare(
    a: &mut RequestArgs,
    op: &str,
    default_timeout: Option<Duration>,
    idle: Option<Duration>,
) -> Result<Prepared, String> {
    let url = a.url.as_str();
    check_url_scheme(url, op)?;
    crate::security::policy::check_http(url)?;
    let parsed = url::Url::parse(url).map_err(|e| op_error(op, format!("invalid URL: {e}")))?;
    let mut net = Net::parse(&a.net, op, default_timeout)?;
    if net.timeout.is_none() && net.connect_timeout.is_none() {
        net.connect_timeout = Some(DEFAULT_CONNECT_TIMEOUT);
    }
    let abort = AbortHandle::register(a.net.abort_id.take(), op)?;
    let mut headers: Vec<(String, String)> = string_headers(a.headers.as_ref())
        .map(|(name, value)| (name.to_owned(), value.to_owned()))
        .collect();
    let body = match a.body.as_ref() {
        None => None,
        Some(body) => Some(encode_body(body, &mut headers, op)?),
    };
    Ok(Prepared {
        agent: net.agent(idle),
        outgoing: Outgoing {
            method: a.method.clone().unwrap_or_else(|| "GET".to_owned()),
            url: parsed,
            headers,
            body,
            progress_id: a.progress_id.clone(),
        },
        net,
        abort,
    })
}

/// Response head as JSON: status, status text, headers, final URL.
fn head_json(exchange: &Exchange) -> serde_json::Map<String, Value> {
    let response = &exchange.response;
    let mut head = serde_json::Map::new();
    head.insert("status".into(), json!(response.status().as_u16()));
    head.insert(
        "statusText".into(),
        json!(response.status().canonical_reason().unwrap_or("")),
    );
    head.insert("headers".into(), Value::Object(response_headers(response)));
    head.insert("url".into(), json!(exchange.url.as_str()));
    head.insert("redirected".into(), json!(exchange.redirected));
    head
}

pub(super) fn request(args: Value) -> Result<Value, String> {
    let mut a: RequestArgs = serde_json::from_value(args)
        .map_err(|e| op_error("request", format!("invalid arguments: {e}")))?;
    let prepared = prepare(&mut a, "request", Some(DEFAULT_TIMEOUT), None)?;
    let mut exchange = execute_watched(
        prepared.agent,
        prepared.outgoing,
        prepared.net,
        &prepared.abort,
        None,
        "request",
    )?;
    let total: Option<u64> =
        header_str(&exchange.response, "content-length").and_then(|s| s.parse::<u64>().ok());
    let mut head = head_json(&exchange);
    let response_type = a.response_type.as_deref().unwrap_or("text");
    let body_bytes = read_response_body(
        &mut exchange.response,
        a.progress_id,
        total,
        &prepared.abort,
    )?;
    head.insert(
        "body".into(),
        decode_response_body(&body_bytes, response_type)?,
    );
    Ok(Value::Object(head))
}

/// Size of one `http:stream` chunk event.
const STREAM_CHUNK: usize = 64 * 1024;
/// Largest `idleTimeoutMs` (24 h).
const MAX_STREAM_IDLE_MS: u64 = 24 * 60 * 60 * 1000;
/// How often a stream waiting for data re-checks its abort flag and caller.
const STREAM_POLL: Duration = Duration::from_millis(100);
/// Slack of the transport read cap over `idleTimeoutMs`, so the stream loop
/// reports the idle timeout itself rather than a bare read error.
const STREAM_READ_SLACK: Duration = Duration::from_secs(1);

/// One read of the body reader thread: bytes, end of body (`None`) or the
/// read error.
type Piece = std::io::Result<Option<Vec<u8>>>;

/// `idleTimeoutMs` of `http.stream`: 1 ms to 24 h, default
/// [`DEFAULT_STREAM_IDLE_TIMEOUT`].
fn stream_idle_timeout(value: Option<u64>) -> Result<Duration, String> {
    match value {
        None => Ok(DEFAULT_STREAM_IDLE_TIMEOUT),
        Some(ms) if (1..=MAX_STREAM_IDLE_MS).contains(&ms) => Ok(Duration::from_millis(ms)),
        Some(ms) => Err(op_error(
            "stream",
            format!("idleTimeoutMs must be 1 to {MAX_STREAM_IDLE_MS}, got {ms}"),
        )),
    }
}

/// `http.stream`: run the request, then emit `http:stream` events to the
/// caller only: `{ id, kind: "head", status, statusText, headers, url,
/// redirected }`, one `{ id, kind: "chunk", data }` (base64) per received
/// block and `{ id, kind: "end", bytes }`. The body is neither buffered nor
/// capped by the host, and there is no overall deadline unless `timeoutMs`
/// is set (long-lived streams such as server-sent events).
///
/// The stream fails once no byte arrived for `idleTimeoutMs` (default
/// [`DEFAULT_STREAM_IDLE_TIMEOUT`]), as soon as it is aborted (even while a
/// read is stalled) and once the calling document is gone (navigation,
/// reload or window close), within [`STREAM_POLL`] / `HEAD_POLL`, also while
/// connecting or waiting for the response head. The request and the body run
/// on helper threads so the call thread can poll for these; a helper stuck in
/// a read ends within about `idleTimeoutMs`, its transport reads being capped
/// too.
pub(super) fn stream(args: Value) -> Result<Value, String> {
    let mut a: RequestArgs = serde_json::from_value(args)
        .map_err(|e| op_error("stream", format!("invalid arguments: {e}")))?;
    let Some(id) = a.stream_id.take() else {
        return Err(op_error("stream", "missing streamId"));
    };
    let valid_id = !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'));
    if !valid_id {
        return Err(op_error("stream", format!("invalid streamId '{id}'")));
    }
    let idle = stream_idle_timeout(a.idle_timeout_ms)?;
    let prepared = prepare(&mut a, "stream", None, Some(idle + STREAM_READ_SLACK))?;
    let owner = crate::ipc::owner::current();
    let emit = |data: &Value| crate::ipc::owner::emit_for(owner.as_ref(), "http:stream", data);
    let exchange = execute_watched(
        prepared.agent,
        prepared.outgoing,
        prepared.net,
        &prepared.abort,
        owner.as_ref(),
        "stream",
    )?;
    let mut head = head_json(&exchange);
    head.insert("id".into(), json!(id));
    head.insert("kind".into(), json!("head"));
    emit(&Value::Object(head));

    let pieces = spawn_body_reader(exchange.response.into_body(), prepared.abort.flag())?;
    let mut total: u64 = 0;
    let mut last_data = Instant::now();
    loop {
        prepared.abort.check("stream")?;
        if owner.as_ref().is_some_and(|owner| !owner.is_current()) {
            return Err(op_error("stream", "the calling document is gone"));
        }
        match pieces.recv_timeout(STREAM_POLL) {
            Ok(Ok(Some(data))) => {
                last_data = Instant::now();
                total += data.len() as u64;
                emit(&json!({ "id": id, "kind": "chunk", "data": STANDARD.encode(&data) }));
            },
            Ok(Ok(None)) => break,
            Ok(Err(e)) => {
                prepared.abort.check("stream")?;
                if last_data.elapsed() >= idle {
                    return Err(idle_error(idle));
                }
                return Err(op_error("stream", format!("read body: {e}")));
            },
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if last_data.elapsed() >= idle {
                    return Err(idle_error(idle));
                }
            },
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err(op_error("stream", "read body: reader stopped"));
            },
        }
    }
    emit(&json!({ "id": id, "kind": "end", "bytes": total }));
    Ok(json!({ "bytes": total }))
}

fn idle_error(idle: Duration) -> String {
    op_error(
        "stream",
        format!(
            "no data received for {} ms (idleTimeoutMs)",
            idle.as_millis()
        ),
    )
}

/// Read `body` on a helper thread, one [`Piece`] per read. The thread ends
/// at the end of the body, on a read error, once `flag` (abort) is seen or
/// once the receiver is dropped.
fn spawn_body_reader(
    body: ureq::Body,
    flag: Arc<AtomicBool>,
) -> Result<mpsc::Receiver<Piece>, String> {
    let (tx, rx) = mpsc::sync_channel::<Piece>(4);
    std::thread::Builder::new()
        .name("tynd-http-stream".into())
        .spawn(move || {
            let mut reader = AbortRead {
                inner: body.into_reader(),
                flag,
            };
            let mut chunk = vec![0_u8; STREAM_CHUNK];
            loop {
                let piece = match reader.read(&mut chunk) {
                    Ok(0) => Ok(None),
                    Ok(read) => Ok(Some(chunk[..read].to_vec())),
                    Err(e) => Err(e),
                };
                let last = !matches!(piece, Ok(Some(_)));
                if tx.send(piece).is_err() || last {
                    return;
                }
            }
        })
        .map_err(|e| op_error("stream", format!("cannot start the body reader: {e}")))?;
    Ok(rx)
}

/// Encode `body`: strings verbatim, anything else as JSON with a default
/// `content-type: application/json` unless the caller set one.
fn encode_body(
    body: &Value,
    headers: &mut Vec<(String, String)>,
    op: &str,
) -> Result<Bytes, String> {
    let bytes = match body {
        Value::String(s) => Bytes::copy_from_slice(s.as_bytes()),
        other => {
            let json_body = serde_json::to_vec(other).map_err(|e| op_error(op, e))?;
            if !headers
                .iter()
                .any(|(name, _)| name.eq_ignore_ascii_case("content-type"))
            {
                headers.push(("content-type".to_owned(), "application/json".to_owned()));
            }
            Bytes::from(json_body)
        },
    };
    if bytes.len() > MAX_REQUEST_BODY {
        return Err(op_error(
            op,
            format!(
                "request body of {} bytes exceeds the {MAX_REQUEST_BODY}-byte cap",
                bytes.len()
            ),
        ));
    }
    Ok(bytes)
}

fn response_headers(response: &Response) -> serde_json::Map<String, Value> {
    let mut headers = serde_json::Map::new();
    for (name, value) in response.headers() {
        let Ok(value) = value.to_str() else {
            continue;
        };
        let key = name.as_str().to_ascii_lowercase();
        match headers.entry(&key) {
            serde_json::map::Entry::Vacant(entry) => {
                entry.insert(Value::String(value.to_string()));
            },
            serde_json::map::Entry::Occupied(mut entry) => {
                let current = entry.get_mut();
                if let Value::Array(values) = current {
                    values.push(Value::String(value.to_string()));
                } else {
                    let previous = mem::replace(current, Value::Null);
                    *current = Value::Array(vec![previous, Value::String(value.to_string())]);
                }
            },
        }
    }
    headers
}

fn read_response_body(
    response: &mut Response,
    progress_id: Option<String>,
    total: Option<u64>,
    abort: &AbortHandle,
) -> Result<Bytes, String> {
    let mut buffer = BytesMut::with_capacity(total.unwrap_or(0).min(8 * 1024 * 1024) as usize);
    let mut reader = AbortRead {
        inner: response.body_mut().as_reader(),
        flag: abort.flag(),
    }
    .take((MAX_RESPONSE_BODY + 1) as u64);
    let mut emitter = ProgressEmitter::new(progress_id, "download", total);
    let mut chunk = vec![0_u8; 64 * 1024];
    loop {
        let read = reader.read(&mut chunk).map_err(|error| {
            if abort.is_aborted() {
                op_error("request", "aborted")
            } else {
                op_error("request", format!("read body: {error}"))
            }
        })?;
        if read == 0 {
            emitter.advance(0, true);
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
        if buffer.len() > MAX_RESPONSE_BODY {
            return Err(op_error(
                "request",
                format!("response body exceeds the {MAX_RESPONSE_BODY}-byte cap"),
            ));
        }
        emitter.advance(read, false);
    }
    Ok(buffer.freeze())
}

fn decode_response_body(body: &Bytes, response_type: &str) -> Result<Value, String> {
    match response_type {
        "binary" => Ok(Value::String(STANDARD.encode(body))),
        "json" => serde_json::from_slice(body).map_err(|error| {
            IpcError::new(
                "http",
                "fetch",
                format!("response is not valid JSON: {error}"),
            )
            .into_string()
        }),
        _ => Ok(Value::String(String::from_utf8_lossy(body).into_owned())),
    }
}
