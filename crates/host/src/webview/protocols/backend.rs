//! Schemes served by backend `app.protocol(name, handler)` handlers.
//!
//! Each webview request is forwarded to the backend over the host/backend
//! channel as a [`ProtocolRequest`] and answered through wry's async
//! responder once the backend replies (`resolve`), fails (`fail`) or
//! [`TIMEOUT`] passes. Bodies are buffered both ways and capped at
//! [`MAX_BODY_BYTES`]: they ride base64-encoded inside one JSON IPC frame.

use std::{
    borrow::Cow,
    collections::HashMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        LazyLock, OnceLock,
    },
    time::{Duration, Instant},
};

use crossbeam_channel::Sender;
use parking_lot::Mutex;
use tynd_protocol::runtime::{BackendCall, ProtocolRequest};
use wry::{
    http::{header, HeaderName, HeaderValue, Method, Request, Response, StatusCode},
    RequestAsyncResponder,
};

use super::{
    forbidden, preflight, request_access, text_response, validate_scheme_name, with_cors,
    ProtocolResponse,
};

/// Request and response body cap. Base64 grows it by a third and the whole
/// frame must stay under `MAX_IPC_FRAME_BYTES` (8 MiB).
pub(crate) const MAX_BODY_BYTES: usize = 4 * 1024 * 1024;
/// How long the backend has to answer before the webview gets a 504.
const TIMEOUT: Duration = Duration::from_secs(30);
/// Most requests waiting on the backend at once, all schemes and webviews
/// together; beyond it the webview gets a 503 instead of holding up to
/// [`TIMEOUT`] and [`MAX_BODY_BYTES`] per request.
const MAX_PENDING: usize = 64;
/// Request ids live in the `__tynd_` namespace the frontend never uses.
const ID_PREFIX: &str = "__tynd_protocol_";

const EXPOSED_HEADERS: &str = "*";

struct Backend {
    schemes: Vec<String>,
    call_tx: Sender<BackendCall>,
}

struct Pending {
    responder: RequestAsyncResponder,
    cors: Option<String>,
    deadline: Instant,
}

static BACKEND: OnceLock<Backend> = OnceLock::new();
static PENDING: LazyLock<Mutex<HashMap<String, Pending>>> = LazyLock::new(Mutex::default);
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// Validate the startup scheme list and keep the channel to the backend.
pub(super) fn configure(schemes: &[String], call_tx: &Sender<BackendCall>) -> Result<(), String> {
    if schemes.is_empty() {
        return Ok(());
    }
    validate_schemes(schemes)?;
    BACKEND
        .set(Backend {
            schemes: schemes.to_vec(),
            call_tx: call_tx.clone(),
        })
        .map_err(|_| "app protocols are already configured".to_owned())?;
    std::thread::Builder::new()
        .name("tynd-protocol-timeout".into())
        .spawn(|| loop {
            std::thread::sleep(Duration::from_secs(1));
            expire(Instant::now());
        })
        .map(drop)
        .map_err(|error| format!("app protocols: cannot start the timeout thread: {error}"))
}

fn validate_schemes(schemes: &[String]) -> Result<(), String> {
    for (index, scheme) in schemes.iter().enumerate() {
        validate_scheme_name(scheme)?;
        if schemes[..index].contains(scheme) {
            return Err(format!("protocol '{scheme}' is registered twice"));
        }
    }
    Ok(())
}

pub(super) fn schemes() -> &'static [String] {
    BACKEND
        .get()
        .map_or(&[], |backend| backend.schemes.as_slice())
}

/// Forward one webview request to the backend handler of `scheme`.
pub(super) fn handle(
    scheme: &str,
    webview: &str,
    request: Request<Vec<u8>>,
    responder: RequestAsyncResponder,
) {
    let Some(backend) = BACKEND.get() else {
        responder.respond(text_response(StatusCode::NOT_FOUND, "unknown protocol"));
        return;
    };
    let Ok(cors) = request_access(&request, webview) else {
        responder.respond(forbidden());
        return;
    };
    if request.method() == Method::OPTIONS {
        responder.respond(preflight(
            &request,
            cors.as_deref(),
            "GET, HEAD, POST, PUT, PATCH, DELETE, OPTIONS",
        ));
        return;
    }
    let request = match protocol_request(scheme, webview, request) {
        Ok(request) => request,
        Err(response) => {
            responder.respond(with_cors(*response, cors.as_deref(), EXPOSED_HEADERS));
            return;
        },
    };
    let id = request.id.clone();
    let admitted = admit(
        &mut PENDING.lock(),
        id.clone(),
        Pending {
            responder,
            cors,
            deadline: Instant::now() + TIMEOUT,
        },
    );
    if let Err(refused) = admitted {
        let response = text_response(
            StatusCode::SERVICE_UNAVAILABLE,
            format!("too many protocol requests in flight (max {MAX_PENDING})"),
        );
        refused.responder.respond(with_cors(
            response,
            refused.cors.as_deref(),
            EXPOSED_HEADERS,
        ));
        return;
    }
    if backend
        .call_tx
        .send(BackendCall::Protocol(Box::new(request)))
        .is_err()
    {
        fail(&id, "backend channel closed");
    }
}

/// Record `entry` as pending under `id` unless [`MAX_PENDING`] requests
/// already wait; the refused entry is handed back so it can be answered.
fn admit<T>(pending: &mut HashMap<String, T>, id: String, entry: T) -> Result<(), T> {
    if pending.len() >= MAX_PENDING {
        return Err(entry);
    }
    pending.insert(id, entry);
    Ok(())
}

/// Convert a wry request, refusing an oversized body.
fn protocol_request(
    scheme: &str,
    webview: &str,
    request: Request<Vec<u8>>,
) -> Result<ProtocolRequest, Box<ProtocolResponse>> {
    if request.body().len() > MAX_BODY_BYTES {
        return Err(Box::new(text_response(
            StatusCode::PAYLOAD_TOO_LARGE,
            format!(
                "protocol request body exceeds {} MiB",
                MAX_BODY_BYTES / (1024 * 1024)
            ),
        )));
    }
    let headers = request
        .headers()
        .iter()
        .filter_map(|(name, value)| {
            value
                .to_str()
                .ok()
                .map(|value| (name.as_str().to_owned(), value.to_owned()))
        })
        .collect();
    let (parts, body) = request.into_parts();
    Ok(ProtocolRequest {
        id: format!("{ID_PREFIX}{}", NEXT_ID.fetch_add(1, Ordering::Relaxed)),
        scheme: scheme.to_owned(),
        method: parts.method.as_str().to_owned(),
        url: parts.uri.to_string(),
        headers,
        body,
        webview: webview.to_owned(),
    })
}

/// Answer the pending request `id` with the backend handler's response.
/// Unknown ids (timed out, or forged) are ignored.
pub(crate) fn resolve(id: &str, status: u16, headers: Vec<(String, String)>, body: Vec<u8>) {
    let Some(pending) = PENDING.lock().remove(id) else {
        return;
    };
    let response = build_response(status, &headers, body);
    pending.responder.respond(with_cors(
        response,
        pending.cors.as_deref(),
        EXPOSED_HEADERS,
    ));
}

/// Answer the pending request `id` with a 502 carrying `message`. Returns
/// whether `id` was a pending protocol request.
pub(crate) fn fail(id: &str, message: &str) -> bool {
    let Some(pending) = PENDING.lock().remove(id) else {
        return false;
    };
    crate::tynd_warn!("app protocol request failed: {message}");
    let response = text_response(
        StatusCode::BAD_GATEWAY,
        format!("protocol handler failed: {message}"),
    );
    pending.responder.respond(with_cors(
        response,
        pending.cors.as_deref(),
        EXPOSED_HEADERS,
    ));
    true
}

/// Answer every request past its deadline with a 504.
fn expire(now: Instant) {
    let expired: Vec<Pending> = {
        let mut pending = PENDING.lock();
        let ids: Vec<String> = pending
            .iter()
            .filter(|(_, entry)| entry.deadline <= now)
            .map(|(id, _)| id.clone())
            .collect();
        ids.iter().filter_map(|id| pending.remove(id)).collect()
    };
    for entry in expired {
        let response = text_response(
            StatusCode::GATEWAY_TIMEOUT,
            format!(
                "protocol handler did not answer within {} s",
                TIMEOUT.as_secs()
            ),
        );
        entry
            .responder
            .respond(with_cors(response, entry.cors.as_deref(), EXPOSED_HEADERS));
    }
}

/// Headers the host owns: the body length, the transport framing and CORS
/// (only the app origin may read a protocol response).
fn host_owned_header(name: &HeaderName) -> bool {
    matches!(
        *name,
        header::CONTENT_LENGTH
            | header::TRANSFER_ENCODING
            | header::CONNECTION
            | header::ACCESS_CONTROL_ALLOW_ORIGIN
            | header::ACCESS_CONTROL_ALLOW_CREDENTIALS
            | header::ACCESS_CONTROL_ALLOW_HEADERS
            | header::ACCESS_CONTROL_ALLOW_METHODS
            | header::ACCESS_CONTROL_EXPOSE_HEADERS
    )
}

fn build_response(status: u16, headers: &[(String, String)], body: Vec<u8>) -> ProtocolResponse {
    if body.len() > MAX_BODY_BYTES {
        return text_response(
            StatusCode::BAD_GATEWAY,
            format!(
                "protocol response body exceeds {} MiB",
                MAX_BODY_BYTES / (1024 * 1024)
            ),
        );
    }
    let Some(status) = (200..=599)
        .contains(&status)
        .then(|| StatusCode::from_u16(status).ok())
        .flatten()
    else {
        return text_response(
            StatusCode::BAD_GATEWAY,
            format!("protocol handler returned an invalid status {status}"),
        );
    };
    let mut response = Response::new(Cow::Owned(body));
    *response.status_mut() = status;
    let map = response.headers_mut();
    for (name, value) in headers {
        let (Ok(name), Ok(value)) = (
            HeaderName::from_bytes(name.as_bytes()),
            HeaderValue::from_str(value),
        ) else {
            continue;
        };
        if !host_owned_header(&name) {
            map.append(name, value);
        }
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scheme_lists_are_validated() {
        assert!(validate_schemes(&["media".into(), "thumbs".into()]).is_ok());
        let twice = validate_schemes(&["media".into(), "media".into()]).unwrap_err();
        assert!(twice.contains("registered twice"), "{twice}");
        assert!(validate_schemes(&["tynd".into()]).is_err());
    }

    #[test]
    fn requests_convert_with_headers_and_body() {
        let request = Request::builder()
            .method("POST")
            .uri("media://localhost/upload?x=1")
            .header("content-type", "text/plain")
            .body(b"payload".to_vec())
            .unwrap();
        let converted = protocol_request("media", "main", request).unwrap();
        assert!(converted.id.starts_with(ID_PREFIX));
        assert_eq!(converted.scheme, "media");
        assert_eq!(converted.method, "POST");
        assert_eq!(converted.url, "media://localhost/upload?x=1");
        assert_eq!(
            converted.headers,
            vec![("content-type".to_owned(), "text/plain".to_owned())]
        );
        assert_eq!(converted.body, b"payload");
        assert_eq!(converted.webview, "main");

        let next = protocol_request(
            "media",
            "main",
            Request::builder()
                .uri("media://localhost/")
                .body(Vec::new())
                .unwrap(),
        )
        .unwrap();
        assert_ne!(next.id, converted.id);
    }

    #[test]
    fn oversized_request_bodies_are_refused() {
        let request = Request::builder()
            .method("POST")
            .uri("media://localhost/")
            .body(vec![0; MAX_BODY_BYTES + 1])
            .unwrap();
        let refused = protocol_request("media", "main", request).unwrap_err();
        assert_eq!(refused.status(), StatusCode::PAYLOAD_TOO_LARGE);
    }

    #[test]
    fn responses_keep_app_headers_and_drop_host_owned_ones() {
        let response = build_response(
            206,
            &[
                ("content-type".into(), "video/mp4".into()),
                ("content-range".into(), "bytes 0-1/10".into()),
                ("set-cookie".into(), "a=1".into()),
                ("set-cookie".into(), "b=2".into()),
                ("content-length".into(), "999".into()),
                ("access-control-allow-origin".into(), "*".into()),
                ("bad header".into(), "x".into()),
            ],
            b"ab".to_vec(),
        );
        assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
        let headers = response.headers();
        assert_eq!(headers["content-type"], "video/mp4");
        assert_eq!(headers["content-range"], "bytes 0-1/10");
        assert_eq!(headers.get_all("set-cookie").iter().count(), 2);
        assert!(headers.get("content-length").is_none());
        assert!(headers.get("access-control-allow-origin").is_none());
        assert_eq!(response.body().as_ref(), b"ab");
    }

    #[test]
    fn invalid_statuses_and_oversized_bodies_become_502() {
        assert_eq!(
            build_response(42, &[], Vec::new()).status(),
            StatusCode::BAD_GATEWAY
        );
        assert_eq!(
            build_response(101, &[], Vec::new()).status(),
            StatusCode::BAD_GATEWAY
        );
        assert_eq!(
            build_response(200, &[], vec![0; MAX_BODY_BYTES + 1]).status(),
            StatusCode::BAD_GATEWAY
        );
    }

    #[test]
    fn in_flight_requests_are_capped() {
        let mut pending = HashMap::new();
        for index in 0..MAX_PENDING {
            assert!(admit(&mut pending, format!("{ID_PREFIX}{index}"), index).is_ok());
        }
        assert_eq!(admit(&mut pending, "over".into(), 999), Err(999));
        assert_eq!(pending.len(), MAX_PENDING);
        // A settled request frees a slot.
        pending.remove(&format!("{ID_PREFIX}0"));
        assert!(admit(&mut pending, "next".into(), 1000).is_ok());
    }

    #[test]
    fn unknown_ids_are_not_pending() {
        assert!(!fail("__tynd_protocol_unknown", "x"));
        resolve("__tynd_protocol_unknown", 200, Vec::new(), Vec::new());
        expire(Instant::now());
    }
}
