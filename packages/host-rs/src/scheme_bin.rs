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
//! Only `fs.readBinary`, `fs.writeBinary`, `compute.hash`, `compute.compress`
//! and `compute.decompress` route through here. Small or text-shaped calls
//! stay on the JSON IPC where they're cheaper and simpler.

use std::borrow::Cow;
use std::collections::HashMap;

use wry::http::{Request, Response, StatusCode};

use crate::os::compute;

type BinResponse = Response<Cow<'static, [u8]>>;

pub fn handle(req: &Request<Vec<u8>>) -> BinResponse {
    let uri = req.uri();
    let path = uri.path().trim_start_matches('/');
    let query = parse_query(uri.query().unwrap_or(""));
    let body = req.body();
    let method = req.method().as_str();

    let Some((api, fn_name)) = path.split_once('/') else {
        return err_response(StatusCode::BAD_REQUEST, format!("bad path: {path}"));
    };

    match (api, fn_name, method) {
        ("fs", "readBinary", "GET") => route_read_binary(&query),
        ("fs", "writeBinary", "POST") => route_write_binary(&query, body),
        ("compute", "hash", "POST") => route_compute_hash(&query, body),
        ("compute", "compress", "POST") => route_compute_compress(&query, body),
        ("compute", "decompress", "POST") => route_compute_decompress(&query, body),
        _ => err_response(
            StatusCode::NOT_FOUND,
            format!("unknown binary route: {method} /{path}"),
        ),
    }
}

fn route_read_binary(query: &HashMap<String, String>) -> BinResponse {
    let Some(path) = query.get("path") else {
        return err_response(StatusCode::BAD_REQUEST, "missing 'path' query".into());
    };
    match std::fs::read(path) {
        Ok(bytes) => ok_bytes(bytes),
        Err(e) => err_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("fs.readBinary({path}): {e}"),
        ),
    }
}

fn route_write_binary(query: &HashMap<String, String>, body: &[u8]) -> BinResponse {
    let Some(path) = query.get("path") else {
        return err_response(StatusCode::BAD_REQUEST, "missing 'path' query".into());
    };
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

fn route_compute_compress(query: &HashMap<String, String>, body: &[u8]) -> BinResponse {
    let algo = query.get("algo").map_or("zstd", String::as_str);
    let level = query
        .get("level")
        .and_then(|s| s.parse::<i32>().ok())
        .unwrap_or(3);
    match compute::compress_raw(body, algo, level) {
        Ok(bytes) => ok_bytes(bytes),
        Err(e) => err_response(StatusCode::BAD_REQUEST, e),
    }
}

fn route_compute_decompress(query: &HashMap<String, String>, body: &[u8]) -> BinResponse {
    let algo = query.get("algo").map_or("zstd", String::as_str);
    match compute::decompress_raw(body, algo) {
        Ok(bytes) => ok_bytes(bytes),
        Err(e) => err_response(StatusCode::BAD_REQUEST, e),
    }
}

fn ok_bytes(bytes: Vec<u8>) -> BinResponse {
    let mut rsp = Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/octet-stream")
        .header("Content-Length", bytes.len().to_string());
    rsp = cache_headers(rsp);
    rsp.body(Cow::Owned(bytes)).unwrap()
}

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

fn parse_query(q: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for pair in q.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        let k = url_decode(k);
        let v = url_decode(v);
        out.insert(k, v);
    }
    out
}

fn url_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            },
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("");
                if let Ok(b) = u8::from_str_radix(hex, 16) {
                    out.push(b);
                    i += 3;
                } else {
                    out.push(bytes[i]);
                    i += 1;
                }
            },
            b => {
                out.push(b);
                i += 1;
            },
        }
    }
    String::from_utf8_lossy(&out).into_owned()
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
        handle(&req)
    }

    fn post(url: &str, body: Vec<u8>) -> BinResponse {
        let req = Request::builder()
            .uri(url)
            .method("POST")
            .body(body)
            .unwrap();
        handle(&req)
    }

    #[test]
    fn unknown_route_is_404() {
        let r = get("tynd-bin://localhost/nope/xxx");
        assert_eq!(r.status(), StatusCode::NOT_FOUND);
    }

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

    #[test]
    fn compute_compress_roundtrip_via_scheme() {
        let payload: Vec<u8> = (0..=255u8).collect();
        let c = post(
            "tynd-bin://localhost/compute/compress?algo=zstd",
            payload.clone(),
        );
        assert_eq!(c.status(), StatusCode::OK);
        let d = post(
            "tynd-bin://localhost/compute/decompress?algo=zstd",
            c.body().to_vec(),
        );
        assert_eq!(d.status(), StatusCode::OK);
        assert_eq!(d.body().as_ref(), payload.as_slice());
    }

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

    #[test]
    fn missing_path_is_400() {
        let r = get("tynd-bin://localhost/fs/readBinary");
        assert_eq!(r.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn url_decode_handles_percent_and_plus() {
        assert_eq!(url_decode("a%20b"), "a b");
        assert_eq!(url_decode("a+b"), "a b");
        assert_eq!(url_decode("C%3A%2Ffoo"), "C:/foo");
    }
}
