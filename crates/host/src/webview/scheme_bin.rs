//! Binary IPC via the `tynd-bin://` custom protocol.
//!
//! The JSON IPC channel (`os_call`) has to base64-encode binary payloads,
//! which costs +33% bytes on the wire plus an encode/decode on both ends.
//! This scheme gives a zero-copy binary path for the handful of APIs that
//! move multi-MB buffers (file IO + compute helpers). Every other call keeps
//! going through the JSON IPC — only APIs that genuinely carry bytes are
//! migrated.
//!
//! Wire format:
//!
//! ```text
//! tynd-bin://localhost/<api>/<method>?<query>
//! body (on POST) = raw input bytes
//! response body  = raw output bytes (or UTF-8 error on non-2xx)
//! ```
//!
//! 4xx/5xx responses have `Content-Type: text/plain; charset=utf-8` and the
//! body is the human-readable error. Successful responses carry whichever
//! content-type the API produces (`application/octet-stream` by default).
//!
//! Only `fs.readBinary`, `fs.writeBinary`, and `compute.hash` route through
//! here. Small or text-shaped calls stay on the JSON IPC where they're
//! cheaper and simpler.
//!
//! The work runs on `crate::ipc::call_pool`, not the UI thread — wry hands us an
//! async `RequestAsyncResponder` so disk IO or multi-MB hashing never
//! blocks paint.

use std::{borrow::Cow, collections::HashMap};

use bytes::Bytes;
use wry::{
    http::{Request, Response, StatusCode},
    RequestAsyncResponder,
};

#[cfg(feature = "api-compute")]
use crate::api::compute;
use crate::ipc::call_pool;

type BinResponse = Response<Cow<'static, [u8]>>;

struct RendererContext {
    window: String,
    webview: String,
}

/// Encode both capability identities into wry's request callback id. Length
/// prefixing avoids separator escaping and supports arbitrary UTF-8 labels.
pub(crate) fn renderer_id(window: &str, webview: &str) -> String {
    format!("{}:{window}{webview}", window.len())
}

fn parse_renderer_id(id: &str) -> Option<RendererContext> {
    let (window_len, labels) = id.split_once(':')?;
    let window_len = window_len.parse::<usize>().ok()?;
    let window = labels.get(..window_len)?;
    let webview = labels.get(window_len..)?;
    if window.is_empty() || webview.is_empty() {
        return None;
    }
    Some(RendererContext {
        window: window.to_string(),
        webview: webview.to_string(),
    })
}

/// Cap on POST body size for binary-scheme routes. Mirrors the matching
/// ceiling on the JSON IPC `http.request` path (`api::http::MAX_REQUEST_BODY`)
/// so neither transport surprises callers with a different OOM line. Anything
/// larger is almost always a runaway loop or a misuse — large file writes
/// should stream in chunks via `fs.open` + repeated `fs.write_handle`.
#[cfg_attr(
    not(any(feature = "api-fs", feature = "api-compute")),
    allow(dead_code)
)]
const MAX_REQUEST_BODY: usize = 100 * 1024 * 1024;

/// Wrap `Bytes` into wry's `Cow<'static, [u8]>` body shape. The internal
/// pipeline carries refcounted `Bytes` so a future fan-out is an Arc clone,
/// not a memcpy.
#[cfg_attr(
    not(any(feature = "api-fs", feature = "api-compute")),
    allow(dead_code)
)]
fn body_from_bytes(b: Bytes) -> Cow<'static, [u8]> {
    Cow::Owned(b.to_vec())
}

/// Async entry point invoked by wry's custom-protocol callback. Parses the
/// route off the UI thread and hands the work to `call_pool` so we never
/// block paint on a big `fs.readBinary` or `compute.compress`.
pub(crate) fn handle_async(
    webview_id: &str,
    req: Request<Vec<u8>>,
    responder: RequestAsyncResponder,
) {
    let context = parse_renderer_id(webview_id);
    let pending = std::sync::Arc::new(parking_lot::Mutex::new(Some((req, responder, context))));
    let worker_pending = std::sync::Arc::clone(&pending);
    if let Err(error) = call_pool::submit(move || {
        if let Some((req, responder, context)) = worker_pending.lock().take() {
            let response = dispatch(&req, context.as_ref());
            responder.respond(response);
        }
    }) {
        if let Some((_req, responder, _context)) = pending.lock().take() {
            responder.respond(err_response(
                StatusCode::SERVICE_UNAVAILABLE,
                format!("binary IPC worker unavailable: {error}"),
            ));
        }
    }
}

/// `Origin` of the request when it is app-local content (the bundled app, its
/// WebView2 `http://tynd.localhost` rewrite or a loopback dev server). Only
/// such an origin is echoed in `Access-Control-Allow-Origin`: remote pages must
/// never be allowed to read binary replies.
fn cors_origin(req: &Request<Vec<u8>>) -> Option<String> {
    let origin = req.headers().get("origin")?.to_str().ok()?;
    super::handlers::is_app_local_url(origin).then(|| origin.to_owned())
}

/// Add CORS headers for an app-local `origin`. The page (`tynd://localhost` or
/// `http://tynd.localhost`) and this scheme are different origins, so the
/// webview enforces CORS on its `fetch` calls.
fn with_cors(mut response: BinResponse, origin: Option<&str>) -> BinResponse {
    use wry::http::HeaderValue;
    let headers = response.headers_mut();
    headers.append("Vary", HeaderValue::from_static("Origin"));
    if let Some(origin) = origin.and_then(|origin| HeaderValue::from_str(origin).ok()) {
        headers.insert("Access-Control-Allow-Origin", origin);
        headers.insert(
            "Access-Control-Expose-Headers",
            HeaderValue::from_static("Content-Type, Content-Length"),
        );
    }
    response
}

/// Answer a CORS preflight. No work is done and nothing is read: the reply only
/// states which methods and headers the real request may use.
fn preflight_response(req: &Request<Vec<u8>>, origin: Option<&str>) -> BinResponse {
    let Some(origin) = origin else {
        return err_response(StatusCode::FORBIDDEN, "cross-origin request refused".into());
    };
    let mut rsp = Response::builder()
        .status(StatusCode::NO_CONTENT)
        .header("Access-Control-Allow-Methods", "GET, POST, OPTIONS")
        .header("Access-Control-Max-Age", "600");
    let requested = req
        .headers()
        .get("access-control-request-headers")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("content-type");
    if let Ok(value) = wry::http::HeaderValue::from_str(requested) {
        rsp = rsp.header("Access-Control-Allow-Headers", value);
    }
    rsp = cache_headers(rsp);
    with_cors(bin_response(rsp, Cow::Borrowed(&[] as &[u8])), Some(origin))
}

fn dispatch(req: &Request<Vec<u8>>, context: Option<&RendererContext>) -> BinResponse {
    let origin = cors_origin(req);
    if req.method() == wry::http::Method::OPTIONS {
        return preflight_response(req, origin.as_deref());
    }
    with_cors(dispatch_route(req, context), origin.as_deref())
}

fn dispatch_route(req: &Request<Vec<u8>>, context: Option<&RendererContext>) -> BinResponse {
    let uri = req.uri();
    let path = uri.path().trim_start_matches('/');
    // `query`/`body` are only consumed when at least one binary route is
    // compiled in. Disabling both `api-fs` and `api-compute` drops them.
    #[cfg_attr(
        not(any(feature = "api-fs", feature = "api-compute")),
        allow(unused_variables)
    )]
    let query = parse_query(uri.query().unwrap_or(""));
    #[cfg_attr(
        not(any(feature = "api-fs", feature = "api-compute")),
        allow(unused_variables)
    )]
    let body = req.body();
    let method = req.method().as_str();

    let Some((api, fn_name)) = path.split_once('/') else {
        return err_response(StatusCode::BAD_REQUEST, format!("bad path: {path}"));
    };
    if let Err(error) = authorize_route(req, context, api, fn_name) {
        return err_response(StatusCode::FORBIDDEN, error);
    }

    match (api, fn_name, method) {
        #[cfg(feature = "api-fs")]
        ("fs", "readBinary", "GET") => route_read_binary(&query),
        #[cfg(feature = "api-fs")]
        ("fs", "writeBinary", "POST") => route_write_binary(&query, body),
        #[cfg(feature = "api-compute")]
        ("compute", "hash", "POST") => route_compute_hash(&query, body),
        _ => err_response(
            StatusCode::NOT_FOUND,
            format!("unknown binary route: {method} /{path}"),
        ),
    }
}

fn authorize_route(
    request: &Request<Vec<u8>>,
    context: Option<&RendererContext>,
    api: &str,
    method: &str,
) -> Result<(), String> {
    let context = context.ok_or_else(|| "binary IPC: missing renderer identity".to_string())?;
    let raw_origin = request
        .headers()
        .get("origin")
        .and_then(|value| value.to_str().ok());
    let fetch_site = request
        .headers()
        .get("sec-fetch-site")
        .and_then(|value| value.to_str().ok());
    if !fetch_site_permitted(raw_origin.is_some(), fetch_site) {
        return Err("binary IPC: cross-site request refused".to_string());
    }
    let origin = super::navigation::ipc_origin(raw_origin, &context.webview);
    let command = format!("{api}:{method}");
    if crate::security::acl::authority().is_allowed(
        &command,
        Some(&context.window),
        Some(&context.webview),
        origin.as_deref(),
    ) {
        Ok(())
    } else {
        Err(format!(
            "renderer command '{command}' denied by capability policy"
        ))
    }
}

/// A request without `Origin` is classified through the webview's main-frame
/// tracking, which a third-party iframe in local content slips past. When the
/// browser says such a request is cross-site (`Sec-Fetch-Site`), refuse it. A
/// missing header (an engine that does not send it) keeps the tracking result.
fn fetch_site_permitted(has_origin: bool, fetch_site: Option<&str>) -> bool {
    has_origin || !fetch_site.is_some_and(|site| site.eq_ignore_ascii_case("cross-site"))
}

#[cfg(feature = "api-fs")]
fn route_read_binary(query: &HashMap<String, String>) -> BinResponse {
    let Some(path) = query.get("path") else {
        return err_response(StatusCode::BAD_REQUEST, "missing 'path' query".into());
    };
    // Policy gate first (deployer allowlist), then `api::fs::resolve` for
    // cap-std sandbox enforcement. Both are mandatory — policy globs alone
    // are circumventable via Win short names / ADS, Unix symlinks.
    if let Err(e) = crate::security::policy::check_fs(path) {
        return err_response(StatusCode::FORBIDDEN, e);
    }
    let (dir, rel) = match crate::api::fs::resolve(path) {
        Ok(p) => p,
        Err(e) => return err_response(StatusCode::FORBIDDEN, e),
    };
    match dir.read(&rel) {
        Ok(bytes) => ok_bytes(Bytes::from(bytes)),
        Err(e) => err_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("fs.readBinary({path}): {e}"),
        ),
    }
}

#[cfg(feature = "api-fs")]
fn route_write_binary(query: &HashMap<String, String>, body: &[u8]) -> BinResponse {
    let Some(path) = query.get("path") else {
        return err_response(StatusCode::BAD_REQUEST, "missing 'path' query".into());
    };
    if body.len() > MAX_REQUEST_BODY {
        return err_response(
            StatusCode::PAYLOAD_TOO_LARGE,
            format!(
                "fs.writeBinary: request body of {} bytes exceeds the {MAX_REQUEST_BODY}-byte cap",
                body.len()
            ),
        );
    }
    if let Err(e) = crate::security::policy::check_fs(path) {
        return err_response(StatusCode::FORBIDDEN, e);
    }
    // SECURITY: cap-std confinement, same as `route_read_binary`.
    let (dir, rel) = match crate::api::fs::resolve(path) {
        Ok(p) => p,
        Err(e) => return err_response(StatusCode::FORBIDDEN, e),
    };
    if query.get("createDirs").map(String::as_str) == Some("1") {
        if let Some(parent) = rel.parent() {
            // Skip `create_dir_all("")` — a syscall on some backends.
            if !parent.as_os_str().is_empty() {
                if let Err(e) = dir.create_dir_all(parent) {
                    return err_response(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!("fs.writeBinary: {e}"),
                    );
                }
            }
        }
    }
    match dir.write(&rel, body) {
        Ok(()) => no_content(),
        Err(e) => err_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("fs.writeBinary({path}): {e}"),
        ),
    }
}

#[cfg(feature = "api-compute")]
fn route_compute_hash(query: &HashMap<String, String>, body: &[u8]) -> BinResponse {
    if body.len() > MAX_REQUEST_BODY {
        return err_response(
            StatusCode::PAYLOAD_TOO_LARGE,
            format!(
                "compute.hash: request body of {} bytes exceeds the {MAX_REQUEST_BODY}-byte cap",
                body.len()
            ),
        );
    }
    let algo = query.get("algo").map_or("blake3", String::as_str);
    let encoding = query.get("encoding").map_or("hex", String::as_str);
    match compute::hash_raw(body, algo, encoding) {
        Ok(s) => {
            let payload = Bytes::from(s.into_bytes());
            let mut rsp = Response::builder()
                .status(StatusCode::OK)
                .header("Content-Type", "text/plain; charset=utf-8");
            rsp = cache_headers(rsp);
            bin_response(rsp, body_from_bytes(payload))
        },
        Err(e) => err_response(StatusCode::BAD_REQUEST, e),
    }
}

#[cfg(feature = "api-fs")]
fn ok_bytes(bytes: Bytes) -> BinResponse {
    let mut rsp = Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/octet-stream")
        .header("Content-Length", bytes.len().to_string());
    rsp = cache_headers(rsp);
    // `body_from_bytes` is the single conversion point to wry's
    // `Cow<'static, [u8]>` API. Internal callers pass `Bytes` so the
    // refcounted buffer flows through any future fan-out (multi-webview
    // broadcast, post-process tap) without a memcpy — the `to_vec()` is
    // the single, final, unavoidable conversion at the wry frontier.
    bin_response(rsp, body_from_bytes(bytes))
}

#[cfg(feature = "api-fs")]
fn no_content() -> BinResponse {
    let mut rsp = Response::builder().status(StatusCode::NO_CONTENT);
    rsp = cache_headers(rsp);
    bin_response(rsp, Cow::Borrowed(&[] as &[u8]))
}

fn err_response(status: StatusCode, message: String) -> BinResponse {
    let mut rsp = Response::builder()
        .status(status)
        .header("Content-Type", "text/plain; charset=utf-8");
    rsp = cache_headers(rsp);
    bin_response(rsp, Cow::Owned(message.into_bytes()))
}

fn bin_response(builder: wry::http::response::Builder, body: Cow<'static, [u8]>) -> BinResponse {
    builder
        .body(body)
        .expect("tynd-bin response uses static status and header values")
}

fn cache_headers(b: wry::http::response::Builder) -> wry::http::response::Builder {
    // Per-call data — never cache. Mirror `scheme.rs` and refuse MIME
    // sniffing so a `text/plain` error body can't be coerced into
    // `text/html` / `application/javascript` by the WebView's content
    // sniffer (which would otherwise let a server-side error string
    // execute as script in the host process's webview).
    b.header("Cache-Control", "no-store")
        .header("X-Content-Type-Options", "nosniff")
}

/// Parse a `tynd-bin://` query string. `form_urlencoded` handles
/// percent-decode, `+`-as-space, UTF-8. Last-wins for repeated keys.
fn parse_query(q: &str) -> HashMap<String, String> {
    form_urlencoded::parse(q.as_bytes())
        .into_owned()
        .collect::<HashMap<_, _>>()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renderer_id_round_trips() {
        let context = parse_renderer_id(&renderer_id("main", "panel")).expect("parse");
        assert_eq!(context.window, "main");
        assert_eq!(context.webview, "panel");
        // Labels may contain the separator and non-ASCII characters.
        let context = parse_renderer_id(&renderer_id("w:1", "vué")).expect("parse");
        assert_eq!(context.window, "w:1");
        assert_eq!(context.webview, "vué");
        assert!(parse_renderer_id("bad").is_none());
        assert!(parse_renderer_id("99:short").is_none());
        assert!(parse_renderer_id("0:webview").is_none());
        assert!(parse_renderer_id("4:main").is_none());
    }

    #[test]
    fn query_is_percent_decoded() {
        let query = parse_query("path=a%20b%2Fc.txt&algo=sha256&x=1&x=2");
        assert_eq!(query["path"], "a b/c.txt");
        assert_eq!(query["algo"], "sha256");
        assert_eq!(query["x"], "2");
        assert!(parse_query("").is_empty());
    }

    fn request(method: &str, origin: Option<&str>) -> Request<Vec<u8>> {
        let mut builder = Request::builder()
            .method(method)
            .uri("tynd-bin://localhost/fs/readBinary?path=x");
        if let Some(origin) = origin {
            builder = builder.header("origin", origin);
        }
        builder.body(Vec::new()).unwrap()
    }

    #[test]
    fn cors_origin_is_echoed_for_app_local_pages_only() {
        let mut locals = vec!["tynd://localhost"];
        if cfg!(target_os = "windows") {
            locals.push("http://tynd.localhost");
        }
        for local in locals {
            assert_eq!(
                cors_origin(&request("GET", Some(local))).as_deref(),
                Some(local)
            );
        }
        // Not the intercepted WebView2 origin: reaches a loopback server.
        for foreign in [
            "http://tynd.localhost:8080",
            "https://tynd.localhost",
            "http://tynd-bin.localhost",
            "http://tynd-bin.localhost:8080",
        ] {
            assert_eq!(
                cors_origin(&request("GET", Some(foreign))),
                None,
                "{foreign}"
            );
        }
        assert_eq!(
            cors_origin(&request("GET", Some("https://evil.test"))),
            None
        );
        assert_eq!(cors_origin(&request("GET", Some("null"))), None);
        // Loopback is app-local only as the configured dev server origin; none
        // is configured in unit tests.
        assert_eq!(
            cors_origin(&request("GET", Some("http://localhost:5173"))),
            None
        );
        assert_eq!(cors_origin(&request("GET", None)), None);
    }

    #[test]
    fn responses_carry_cors_headers_for_local_origins() {
        let response = with_cors(
            err_response(StatusCode::BAD_REQUEST, "x".into()),
            Some("tynd://localhost"),
        );
        let headers = response.headers();
        assert_eq!(headers["access-control-allow-origin"], "tynd://localhost");
        assert_eq!(headers["vary"], "Origin");
        let denied = with_cors(err_response(StatusCode::BAD_REQUEST, "x".into()), None);
        assert!(!denied.headers().contains_key("access-control-allow-origin"));
    }

    #[test]
    fn preflight_is_answered_only_for_local_origins() {
        let local = request("OPTIONS", Some("tynd://localhost"));
        let response = dispatch(&local, None);
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        assert_eq!(
            response.headers()["access-control-allow-origin"],
            "tynd://localhost"
        );
        assert!(response.headers()["access-control-allow-methods"]
            .to_str()
            .unwrap()
            .contains("POST"));
        let remote = request("OPTIONS", Some("https://evil.test"));
        let response = dispatch(&remote, None);
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert!(!response
            .headers()
            .contains_key("access-control-allow-origin"));
    }

    #[test]
    fn cross_site_requests_without_origin_are_refused() {
        assert!(!fetch_site_permitted(false, Some("cross-site")));
        assert!(!fetch_site_permitted(false, Some("Cross-Site")));
        assert!(fetch_site_permitted(false, Some("same-origin")));
        assert!(fetch_site_permitted(false, Some("none")));
        assert!(fetch_site_permitted(false, None));
        // With an Origin the ACL origin check already applies.
        assert!(fetch_site_permitted(true, Some("cross-site")));
    }
}
