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
//! The work runs on `os::call_pool`, not the UI thread — wry hands us an
//! async `RequestAsyncResponder` so disk IO or multi-MB hashing never
//! blocks paint.

use std::borrow::Cow;
use std::collections::HashMap;

use wry::http::{Request, Response, StatusCode};
use wry::RequestAsyncResponder;

use crate::os::call_pool;
#[cfg(feature = "api-compute")]
use crate::os::compute;

type BinResponse = Response<Cow<'static, [u8]>>;

/// Async entry point invoked by wry's custom-protocol callback. Parses the
/// route off the UI thread and hands the work to `call_pool` so we never
/// block paint on a big `fs.readBinary` or `compute.compress`.
pub fn handle_async(req: Request<Vec<u8>>, responder: RequestAsyncResponder) {
    call_pool::submit(move || {
        let response = dispatch(&req);
        responder.respond(response);
    });
}

fn dispatch(req: &Request<Vec<u8>>) -> BinResponse {
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

#[cfg(feature = "api-fs")]
fn route_read_binary(query: &HashMap<String, String>) -> BinResponse {
    let Some(path) = query.get("path") else {
        return err_response(StatusCode::BAD_REQUEST, "missing 'path' query".into());
    };
    if let Err(e) = crate::os::security::check_fs(path) {
        return err_response(StatusCode::FORBIDDEN, e);
    }
    match std::fs::read(path) {
        Ok(bytes) => ok_bytes(bytes),
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
    if let Err(e) = crate::os::security::check_fs(path) {
        return err_response(StatusCode::FORBIDDEN, e);
    }
    if query.get("createDirs").map(String::as_str) == Some("1") {
        if let Some(parent) = std::path::Path::new(path).parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                return err_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("fs.writeBinary: {e}"),
                );
            }
        }
    }
    match std::fs::write(path, body) {
        Ok(()) => no_content(),
        Err(e) => err_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("fs.writeBinary({path}): {e}"),
        ),
    }
}

#[cfg(feature = "api-compute")]
fn route_compute_hash(query: &HashMap<String, String>, body: &[u8]) -> BinResponse {
    let algo = query.get("algo").map_or("blake3", String::as_str);
    let encoding = query.get("encoding").map_or("hex", String::as_str);
    match compute::hash_raw(body, algo, encoding) {
        Ok(s) => {
            let mut rsp = Response::builder()
                .status(StatusCode::OK)
                .header("Content-Type", "text/plain; charset=utf-8");
            rsp = cache_headers(rsp);
            rsp.body(Cow::Owned(s.into_bytes())).unwrap()
        },
        Err(e) => err_response(StatusCode::BAD_REQUEST, e),
    }
}

#[cfg(feature = "api-fs")]
fn ok_bytes(bytes: Vec<u8>) -> BinResponse {
    let mut rsp = Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/octet-stream")
        .header("Content-Length", bytes.len().to_string());
    rsp = cache_headers(rsp);
    rsp.body(Cow::Owned(bytes)).unwrap()
}

#[cfg(feature = "api-fs")]
fn no_content() -> BinResponse {
    let mut rsp = Response::builder().status(StatusCode::NO_CONTENT);
    rsp = cache_headers(rsp);
    rsp.body(Cow::Borrowed(&[] as &[u8])).unwrap()
}

fn err_response(status: StatusCode, message: String) -> BinResponse {
    let mut rsp = Response::builder()
        .status(status)
        .header("Content-Type", "text/plain; charset=utf-8");
    rsp = cache_headers(rsp);
    // The body is owned so it survives `'static` without heap gymnastics.
    rsp.body(Cow::Owned(message.into_bytes())).unwrap()
}

fn cache_headers(b: wry::http::response::Builder) -> wry::http::response::Builder {
    // These responses carry per-call data — never cache.
    b.header("Cache-Control", "no-store")
}

/// Parse a `tynd-bin://` query string into a typed map.
///
/// `form_urlencoded` does the percent-decode + `+`-to-space + UTF-8
/// validation — replaces a hand-rolled state machine that silently
/// substituted `U+FFFD` on any non-UTF-8 byte run. Last-wins semantics
/// mirror the previous behaviour: collecting into `HashMap` lets the
/// final occurrence of a repeated key clobber earlier ones.
fn parse_query(q: &str) -> HashMap<String, String> {
    form_urlencoded::parse(q.as_bytes())
        .into_owned()
        .collect::<HashMap<_, _>>()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn get(url: &str) -> BinResponse {
        let req = Request::builder()
            .uri(url)
            .method("GET")
            .body(Vec::new())
            .unwrap();
        dispatch(&req)
    }

    #[cfg(any(feature = "api-fs", feature = "api-compute"))]
    fn post(url: &str, body: Vec<u8>) -> BinResponse {
        let req = Request::builder()
            .uri(url)
            .method("POST")
            .body(body)
            .unwrap();
        dispatch(&req)
    }

    #[test]
    fn unknown_route_is_404() {
        let r = get("tynd-bin://localhost/nope/xxx");
        assert_eq!(r.status(), StatusCode::NOT_FOUND);
    }

    #[cfg(feature = "api-fs")]
    #[test]
    fn fs_read_binary_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("raw.bin");
        let bytes: Vec<u8> = (0..=255u8).collect();
        std::fs::write(&path, &bytes).unwrap();
        let url = format!("tynd-bin://localhost/fs/readBinary?path={}", path.display());
        let r = get(&url);
        assert_eq!(r.status(), StatusCode::OK);
        assert_eq!(r.body().as_ref(), bytes.as_slice());
    }

    #[cfg(feature = "api-fs")]
    #[test]
    fn fs_write_binary_writes_body() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.bin");
        let body: Vec<u8> = vec![9, 8, 7, 6];
        let url = format!(
            "tynd-bin://localhost/fs/writeBinary?path={}",
            path.display()
        );
        let r = post(&url, body.clone());
        assert_eq!(r.status(), StatusCode::NO_CONTENT);
        assert_eq!(std::fs::read(&path).unwrap(), body);
    }

    #[cfg(feature = "api-compute")]
    #[test]
    fn compute_hash_returns_hex_string() {
        let r = post(
            "tynd-bin://localhost/compute/hash?algo=sha256&encoding=hex",
            b"abc".to_vec(),
        );
        assert_eq!(r.status(), StatusCode::OK);
        let s = std::str::from_utf8(r.body()).unwrap();
        assert_eq!(
            s,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[cfg(feature = "api-fs")]
    #[test]
    fn missing_path_is_400() {
        let r = get("tynd-bin://localhost/fs/readBinary");
        assert_eq!(r.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn parse_query_handles_percent_and_plus() {
        // `form_urlencoded` covers the same ground as the old
        // `url_decode` helper — keep an integration check so a future
        // dep upgrade doesn't silently regress percent-decoding or
        // `+`-as-space mapping.
        let q = parse_query("k=a%20b&j=a+b&c=C%3A%2Ffoo");
        assert_eq!(q.get("k").map(String::as_str), Some("a b"));
        assert_eq!(q.get("j").map(String::as_str), Some("a b"));
        assert_eq!(q.get("c").map(String::as_str), Some("C:/foo"));
    }
}
