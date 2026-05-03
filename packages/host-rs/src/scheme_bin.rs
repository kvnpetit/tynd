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

use std::{borrow::Cow, collections::HashMap};

use bytes::Bytes;
use wry::{
    http::{Request, Response, StatusCode},
    RequestAsyncResponder,
};

use crate::os::call_pool;
#[cfg(feature = "api-compute")]
use crate::os::compute;

type BinResponse = Response<Cow<'static, [u8]>>;

/// Wrap `Bytes` into the `Cow<'static, [u8]>` shape wry currently demands at
/// the custom-protocol boundary. Internally the bytes ride through the
/// pipeline as a refcounted `Bytes` (so any future fan-out — multi-webview
/// broadcast, post-process pipeline — clones an `Arc` inc instead of memcpy
/// of the whole payload). The final `to_vec()` is the unavoidable conversion
/// to wry's API; if/when wry accepts `Bytes` directly the win cascades to the
/// frontier with no further changes here.
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
    // Policy first (fast reject on glob deny / no-match-with-default-deny)
    // BEFORE the kernel-side cap-std `openat2` call. Both checks are
    // mandatory — policy is the deployer-visible allowlist, cap-std is
    // the OS-enforced sandbox.
    if let Err(e) = crate::os::security::check_fs(path) {
        return err_response(StatusCode::FORBIDDEN, e);
    }
    // SECURITY: route through `os::fs::resolve` so this binary path goes
    // through the same cap-std `Dir` confinement as the JSON `fs.readText`
    // API. Direct `std::fs::read(path)` bypasses the sandbox and leaves
    // only the policy globs as defense — those are circumventable on
    // Windows via short names / ADS, on Unix via symlinks.
    let (dir, rel) = match crate::os::fs::resolve(path) {
        Ok(p) => p,
        Err(e) => return err_response(StatusCode::FORBIDDEN, e),
    };
    match dir.read(&rel) {
        // Wrap the freshly-read Vec in `Bytes` so the response pipeline
        // holds a refcounted handle. Today there's a single subscriber
        // (the wry response), but the type is now ready for fan-out
        // (multi-webview broadcast, future post-process tap) without a
        // memcpy: `Bytes::clone()` is an Arc inc.
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
    if let Err(e) = crate::os::security::check_fs(path) {
        return err_response(StatusCode::FORBIDDEN, e);
    }
    // SECURITY: same justification as `route_read_binary` — write must
    // go through cap-std so the kernel rejects any path that escapes the
    // pre-opened roots (symlink, `..`, absolute-path injection).
    let (dir, rel) = match crate::os::fs::resolve(path) {
        Ok(p) => p,
        Err(e) => return err_response(StatusCode::FORBIDDEN, e),
    };
    if query.get("createDirs").map(String::as_str) == Some("1") {
        if let Some(parent) = rel.parent() {
            // `Dir::create_dir_all("")` is a no-op on some backends but a
            // syscall on others — skip when the file is at the Dir root.
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
    let algo = query.get("algo").map_or("blake3", String::as_str);
    let encoding = query.get("encoding").map_or("hex", String::as_str);
    match compute::hash_raw(body, algo, encoding) {
        Ok(s) => {
            // Hex/base64 digest is small (<= 128 B) but we still go through
            // `Bytes` so the response-building path stays uniform with the
            // binary-payload routes above.
            let payload = Bytes::from(s.into_bytes());
            let mut rsp = Response::builder()
                .status(StatusCode::OK)
                .header("Content-Type", "text/plain; charset=utf-8");
            rsp = cache_headers(rsp);
            rsp.body(body_from_bytes(payload)).unwrap()
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
    rsp.body(body_from_bytes(bytes)).unwrap()
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
        // Hold the security policy lock — sibling tests below mutate
        // the global policy via `configure(...)`. Without this the
        // 403-returning policy from a parallel test races with the
        // round-trip and we get a flaky FORBIDDEN where 200 is expected.
        let _g = lock();
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
        // Same race-protection as `fs_read_binary_round_trips` above.
        let _g = lock();
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

    #[cfg(feature = "api-fs")]
    #[test]
    fn fs_read_binary_rejects_empty_path() {
        // Empty path bypasses both the policy globs and `os::fs::resolve`.
        // `resolve` short-circuits on the empty input — exercise the path
        // to confirm the error surfaces as 4xx (FORBIDDEN/BAD_REQUEST) and
        // not 500 from a `std::fs::read("")` blowup.
        let r = get("tynd-bin://localhost/fs/readBinary?path=");
        assert!(
            r.status() == StatusCode::FORBIDDEN || r.status() == StatusCode::BAD_REQUEST,
            "expected 4xx for empty path, got {}",
            r.status()
        );
    }

    // -----------------------------------------------------------------
    // W4.3 — binary scheme policy + cap-std coverage
    //
    // The two routes below already get the happy-path round-trip in the
    // existing `fs_read_binary_round_trips` / `fs_write_binary_writes_body`
    // tests. The additions here focus on:
    //   * policy fast-reject BEFORE any disk syscall (deny-glob match)
    //   * cap-std layer rejecting an absolute path outside every
    //     default root (i.e. resolve() returns Err → 403)
    //   * write_binary creating + writing OK under a policy that allows
    //     the destination, as a sanity round-trip with policy active.
    // -----------------------------------------------------------------

    #[cfg(feature = "api-fs")]
    fn lock() -> std::sync::MutexGuard<'static, ()> {
        crate::os::security::test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    #[cfg(feature = "api-fs")]
    #[test]
    fn fs_read_binary_pre_check_policy_fast_rejects() {
        // The policy check runs BEFORE `os::fs::resolve` touches disk.
        // Configure a deny rule that matches by glob, then hit the
        // route with a path that doesn't even exist on disk — the
        // response must still be 403, proving the policy gate fired
        // ahead of any IO. (If the file existed, this could mask a
        // bug where the IO layer happens to fail with the same status;
        // using a definitely-not-real filename makes the assertion
        // unambiguous.)
        let _g = lock();
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let denied_path = std::env::temp_dir().join(format!("tynd-deny-{nonce}.bin"));
        let denied_str = denied_path.to_string_lossy().replace('\\', "/");
        let pattern = if cfg!(target_os = "windows") {
            format!("{}/**", denied_str.to_lowercase().rsplit_once('/').unwrap().0)
        } else {
            format!("{}/**", denied_str.rsplit_once('/').unwrap().0)
        };

        crate::os::security::dispatch(
            "configure",
            &serde_json::json!({
                "fs": { "deny": [pattern] },
                "defaultDeny": false,
            }),
        )
        .unwrap();

        let url = format!(
            "tynd-bin://localhost/fs/readBinary?path={}",
            denied_path.display()
        );
        let r = get(&url);
        assert_eq!(r.status(), StatusCode::FORBIDDEN);

        // Reset policy so neighbouring tests aren't poisoned.
        crate::os::security::dispatch(
            "configure",
            &serde_json::json!({ "fs": { "allow": [] }, "defaultDeny": false }),
        )
        .unwrap();
    }

    #[cfg(feature = "api-fs")]
    #[test]
    fn fs_read_binary_resolve_rejects_path_outside_roots() {
        // No policy configured — but `os::fs::resolve` still has to
        // reject empty paths (and, in principle, paths outside every
        // default root). Empty is the portable "outside scope" we can
        // construct without poking volumes that don't exist on every
        // CI runner. The route should return 403 from the resolve
        // layer rather than 500 from a `std::fs::read("")` blowup.
        let _g = lock();
        let r = get("tynd-bin://localhost/fs/readBinary?path=");
        assert!(
            r.status() == StatusCode::FORBIDDEN
                || r.status() == StatusCode::BAD_REQUEST,
            "expected 4xx from resolve/policy, got {}",
            r.status()
        );
    }

    #[cfg(feature = "api-fs")]
    #[test]
    fn fs_write_binary_with_policy_allow_succeeds() {
        // Round-trip with policy active: configure an allow scope under
        // tempdir, write via the binary route, then read back via std
        // and confirm the bytes landed. Catches a regression where
        // the write path silently bypasses the cap-std `Dir::write`
        // (e.g. by reverting to `std::fs::write`).
        let _g = lock();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("written.bin");
        let path_str = path.to_string_lossy().replace('\\', "/");
        let allow_pattern = if cfg!(target_os = "windows") {
            format!("{}/**", dir.path().to_string_lossy()
                .replace('\\', "/")
                .to_lowercase())
        } else {
            format!("{}/**", dir.path().display())
        };

        crate::os::security::dispatch(
            "configure",
            &serde_json::json!({
                "fs": { "allow": [allow_pattern] },
                "defaultDeny": true,
            }),
        )
        .unwrap();

        let body = vec![1u8, 2, 3, 4, 5];
        let url = format!("tynd-bin://localhost/fs/writeBinary?path={path_str}");
        let r = post(&url, body.clone());
        assert_eq!(r.status(), StatusCode::NO_CONTENT);
        assert_eq!(std::fs::read(&path).unwrap(), body);

        crate::os::security::dispatch(
            "configure",
            &serde_json::json!({ "fs": { "allow": [] }, "defaultDeny": false }),
        )
        .unwrap();
    }
}
