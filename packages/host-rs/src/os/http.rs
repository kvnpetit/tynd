use std::{
    fs,
    io::{Read, Write},
    time::{Duration, Instant},
};

use base64::{engine::general_purpose::STANDARD, Engine};
use bytes::{Bytes, BytesMut};
use serde::Deserialize;
use serde_json::{json, Value};

use super::events;
use crate::ipc_error::IpcError;

// ─── Typed argument shapes ──────────────────────────────────────────────
//
// Each endpoint that takes 3+ arguments declares its own
// `#[derive(Deserialize)]` struct here. `serde_json::from_value` does
// the field-by-field extraction in one pass — replaces the
// `args.get("foo").and_then(Value::as_str).ok_or_else(...)` quartet
// repeated 4-7 times per endpoint with a single typed deserialization.
//
// Notes on the shape:
//   * `#[serde(default)]` on optional fields keeps the legacy
//     "missing == None / default" semantics so we don't break callers
//     that omit fields.
//   * `#[serde(alias = "...")]` covers the snake_case ↔ camelCase
//     duality the old code papered over with `args.get("file_path")
//     .or_else(|| args.get("filePath"))` chains.
//   * Bodies / headers stay as `Value` because they're forwarded to
//     `apply_headers` / the request-body branch as-is — typing them
//     would force a clone with no payoff.
//
// This pattern is the recommended template for any new endpoint that
// pulls 3+ fields out of the args bag — see commit message for the
// migration recipe.

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RequestArgs {
    url: String,
    #[serde(default)]
    method: Option<String>,
    #[serde(default)]
    timeout_ms: Option<u64>,
    #[serde(default)]
    progress_id: Option<String>,
    #[serde(default)]
    headers: Option<Value>,
    #[serde(default)]
    body: Option<Value>,
    #[serde(default)]
    response_type: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DownloadArgs {
    url: String,
    dest: String,
    #[serde(default)]
    timeout_ms: Option<u64>,
    #[serde(default)]
    progress_id: Option<String>,
    #[serde(default)]
    headers: Option<Value>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UploadArgs {
    url: String,
    // Accept the legacy snake_case `file_path` too so the IPC contract
    // stays byte-compatible during the rollout — JS callers send
    // `filePath` (the camelCase rename), but Rust unit tests pass
    // `file_path` directly.
    #[serde(alias = "file_path")]
    file_path: String,
    #[serde(default, alias = "field_name")]
    field_name: Option<String>,
    #[serde(default)]
    timeout_ms: Option<u64>,
    #[serde(default)]
    progress_id: Option<String>,
    #[serde(default)]
    headers: Option<Value>,
}

pub fn dispatch(method: &str, args: &Value) -> Result<Value, String> {
    match method {
        "request" => request(args),
        "download" => download(args),
        "upload" => upload(args),
        _ => Err(IpcError::unknown_method("http", method).into_string()),
    }
}

/// Build a ureq 3 `Agent` with our standard timeout + status-code policy.
///
/// `http_status_as_error = false` keeps 4xx/5xx responses flowing through
/// the normal `Ok(response)` path so the IPC layer can surface the body
/// to JS the same way as a 200 — matches the 2.x semantics our callers
/// rely on (the lite `fetch` polyfill's "still iterable on error status"
/// branch in particular).
///
/// When the `tls-platform-verifier` host feature is on we flip the root
/// certs over to `RootCerts::PlatformVerifier` so corp / enterprise CA
/// chains resolve through the OS trust store (Keychain / Win-crypto /
/// NSS). The default rustls webpki root set stays in place otherwise.
fn build_agent(timeout_ms: Option<u64>) -> ureq::Agent {
    let timeout = timeout_ms.map_or(Duration::from_secs(60), Duration::from_millis);
    #[allow(unused_mut)]
    let mut config = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(timeout));

    #[cfg(feature = "tls-platform-verifier")]
    {
        use ureq::tls::{RootCerts, TlsConfig};
        config = config.tls_config(
            TlsConfig::builder()
                .root_certs(RootCerts::PlatformVerifier)
                .build(),
        );
    }

    config.build().new_agent()
}

/// Apply caller-supplied headers to an `http::request::Builder`. Used by
/// the `request()` path which assembles an `http::Request` and hands it
/// to `Agent::run` for the arbitrary-method dispatch ureq 3's typed
/// helpers don't cover (`agent.get/post/put/...` shadow the verb but
/// the runtime caller chooses the verb dynamically).
fn apply_headers_http(
    mut req: ureq::http::request::Builder,
    headers: Option<&Value>,
) -> ureq::http::request::Builder {
    if let Some(obj) = headers.and_then(Value::as_object) {
        for (k, v) in obj {
            if let Some(s) = v.as_str() {
                req = req.header(k, s);
            }
        }
    }
    req
}

/// Throttled to ≥50ms between emits so fast streams don't flood IPC.
struct ProgressEmitter {
    id: Option<String>,
    phase: &'static str,
    total: Option<u64>,
    loaded: u64,
    last_emit: Instant,
}

impl ProgressEmitter {
    fn new(id: Option<String>, phase: &'static str, total: Option<u64>) -> Self {
        Self {
            id,
            phase,
            total,
            loaded: 0,
            last_emit: Instant::now()
                .checked_sub(Duration::from_secs(1))
                .unwrap_or_else(Instant::now),
        }
    }

    fn advance(&mut self, delta: usize, force: bool) {
        let Some(id) = self.id.as_ref() else { return };
        self.loaded += delta as u64;
        if !force && self.last_emit.elapsed() < Duration::from_millis(50) {
            return;
        }
        self.last_emit = Instant::now();
        events::emit(
            "http:progress",
            &json!({
                "id": id,
                "phase": self.phase,
                "loaded": self.loaded,
                "total": self.total,
            }),
        );
    }
}

struct ProgressReader {
    inner: std::io::Cursor<Vec<u8>>,
    emitter: ProgressEmitter,
}

impl Read for ProgressReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        if n == 0 {
            self.emitter.advance(0, true);
        } else {
            self.emitter.advance(n, false);
        }
        Ok(n)
    }
}

/// Read a header value as a UTF-8 string. ureq 3's `headers().get()`
/// returns a `&HeaderValue` whose `to_str()` errors on non-ASCII —
/// match 2.x behaviour by silently dropping non-UTF8 headers (we only
/// surface them to JS as JSON strings anyway).
fn header_str<'a>(resp: &'a ureq::http::Response<ureq::Body>, name: &str) -> Option<&'a str> {
    resp.headers().get(name).and_then(|v| v.to_str().ok())
}

fn request(args: &Value) -> Result<Value, String> {
    let a: RequestArgs = serde_json::from_value(args.clone()).map_err(|e| {
        IpcError::new("http", "request", format!("invalid arguments: {e}")).into_string()
    })?;
    let url = a.url.as_str();
    super::security::check_http(url)?;
    let method = a.method.as_deref().unwrap_or("GET");
    let timeout_ms = a.timeout_ms;
    let progress_id = a.progress_id.clone();

    let agent = build_agent(timeout_ms);

    // ureq 3 doesn't expose a generic `agent.request(method, url)` —
    // the typed `agent.get/post/put/...` helpers shadow the verb at
    // compile time. For our case the verb is runtime-driven (chosen
    // by the JS caller), so we hand-build an `http::Request` and run
    // it through `agent.run(...)`. Same agent, same middleware, just
    // bypassing the typestate sugar.
    let mut http_builder = ureq::http::Request::builder()
        .method(method)
        .uri(url);
    http_builder = apply_headers_http(http_builder, a.headers.as_ref());

    let response = if let Some(body) = a.body.as_ref() {
        let (body_bytes, content_type_set) = match body {
            Value::String(s) => (s.as_bytes().to_vec(), true),
            other => {
                let json_body = serde_json::to_vec(other).map_err(|e| e.to_string())?;
                let has_ct = http_builder
                    .headers_ref()
                    .map(|h| h.keys().any(|k| k.as_str().eq_ignore_ascii_case("content-type")))
                    .unwrap_or(false);
                (json_body, has_ct)
            },
        };
        if !content_type_set {
            http_builder = http_builder.header("content-type", "application/json");
        }
        if progress_id.is_some() {
            // Streaming-with-progress path: wrap the body bytes in a
            // `ProgressReader` and route through `Agent::run` after
            // turning the builder into a `Request<SendBody>`. The
            // emitter ticks happen as ureq pumps the reader.
            let total = Some(body_bytes.len() as u64);
            let mut reader = ProgressReader {
                inner: std::io::Cursor::new(body_bytes),
                emitter: ProgressEmitter::new(progress_id.clone(), "upload", total),
            };
            let req = http_builder
                .body(ureq::SendBody::from_reader(&mut reader))
                .map_err(|e| {
                    IpcError::new("http", "request", format!("build request: {e}"))
                        .into_string()
                })?;
            agent.run(req)
        } else {
            // Wrap in `Cursor<Vec<u8>>` and use `from_owned_reader` so
            // the body owns its bytes for the duration of the call —
            // ureq 3 doesn't have a direct `from_owned_vec`. The cursor
            // is `Read` over the vec; same wire effect as `send_bytes`.
            let req = http_builder
                .body(ureq::SendBody::from_owned_reader(std::io::Cursor::new(body_bytes)))
                .map_err(|e| {
                    IpcError::new("http", "request", format!("build request: {e}"))
                        .into_string()
                })?;
            agent.run(req)
        }
    } else {
        let req = http_builder.body(ureq::SendBody::none()).map_err(|e| {
            IpcError::new("http", "request", format!("build request: {e}")).into_string()
        })?;
        agent.run(req)
    };

    let mut resp = response
        .map_err(|e| IpcError::new("http", "request", format!("({url}): {e}")).into_string())?;
    let status = resp.status().as_u16();
    let status_text = resp
        .status()
        .canonical_reason()
        .unwrap_or("")
        .to_string();
    let total: Option<u64> = header_str(&resp, "content-length")
        .and_then(|s| s.parse::<u64>().ok());
    let mut headers_map = serde_json::Map::new();
    for (name, value) in resp.headers() {
        if let Ok(v) = value.to_str() {
            headers_map.insert(name.as_str().to_lowercase(), Value::String(v.to_string()));
        }
    }

    let response_type = a.response_type.as_deref().unwrap_or("text");

    // Accumulate the response body in a `BytesMut` so we get amortized
    // power-of-two growth backed by a single allocation, then `freeze()`
    // it into an immutable `Bytes`. The downstream encode/parse calls
    // (base64 / JSON / UTF-8) all consume `&[u8]` which `Bytes` derefs
    // to — no extra copy. If a future code path wants to fan the
    // response out (e.g. emit it on the event bus *and* return it from
    // the call), `Bytes::clone()` is an Arc inc instead of a memcpy of
    // the whole body.
    let mut buf = BytesMut::with_capacity(total.unwrap_or(0).min(8 * 1024 * 1024) as usize);
    // ureq 3's `body_mut().as_reader()` is unbounded by default — bring
    // the same 100 MiB cap the 2.x `take(...)` chain enforced.
    let mut reader = resp.body_mut().as_reader().take(100 * 1024 * 1024);
    let mut emitter = ProgressEmitter::new(progress_id, "download", total);
    let mut chunk = vec![0u8; 64 * 1024];
    loop {
        let n = reader
            .read(&mut chunk)
            .map_err(|e| IpcError::new("http", "request", format!("read body: {e}")).into_string())?;
        if n == 0 {
            emitter.advance(0, true);
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
        emitter.advance(n, false);
    }
    let buf: Bytes = buf.freeze();

    let body = match response_type {
        "binary" => Value::String(STANDARD.encode(&buf)),
        "json" => serde_json::from_slice::<Value>(&buf).unwrap_or(Value::Null),
        _ => Value::String(String::from_utf8_lossy(&buf).into_owned()),
    };

    Ok(json!({
        "status": status,
        "statusText": status_text,
        "headers": Value::Object(headers_map),
        "body": body,
    }))
}

fn download(args: &Value) -> Result<Value, String> {
    let a: DownloadArgs = serde_json::from_value(args.clone()).map_err(|e| {
        IpcError::new("http", "download", format!("invalid arguments: {e}")).into_string()
    })?;
    let url = a.url.as_str();
    super::security::check_http(url)?;
    let dest = a.dest.as_str();
    super::security::check_fs(dest)?;
    let timeout_ms = a.timeout_ms;
    let progress_id = a.progress_id;

    let agent = build_agent(timeout_ms);
    let mut req_builder = agent.get(url);
    if let Some(obj) = a.headers.as_ref().and_then(Value::as_object) {
        for (k, v) in obj {
            if let Some(s) = v.as_str() {
                req_builder = req_builder.header(k, s);
            }
        }
    }
    let mut resp = req_builder
        .call()
        .map_err(|e| IpcError::new("http", "download", format!("({url}): {e}")).into_string())?;

    let total: Option<u64> = header_str(&resp, "content-length")
        .and_then(|s| s.parse::<u64>().ok());

    if let Some(parent) = std::path::Path::new(dest).parent() {
        fs::create_dir_all(parent).map_err(|e| {
            IpcError::new("http", "download", format!("mkdir {}: {e}", parent.display()))
                .into_string()
        })?;
    }

    // 10 GiB cap — large enough for any reasonable update binary while still
    // bounding pathological / hostile responses (servers without
    // content-length).
    const MAX_DOWNLOAD: u64 = 10 * 1024 * 1024 * 1024;
    if let Some(t) = total {
        if t > MAX_DOWNLOAD {
            return Err(IpcError::new(
                "http",
                "download",
                format!("response size {t} exceeds the {MAX_DOWNLOAD}-byte cap"),
            )
            .into_string());
        }
    }

    // Atomic write via `tempfile::NamedTempFile`. Streams into a
    // sibling temp file in `dest`'s parent so the rename at the end is
    // a same-volume move; on Windows `persist` calls `ReplaceFileW`
    // (atomic rename-if-exists), which the manual `tmp-<pid>-<nanos>`
    // dance the previous code did couldn't guarantee. Drop on the
    // temp file removes it automatically when an error path returns.
    let parent = std::path::Path::new(dest)
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map_or_else(
            || std::path::PathBuf::from("."),
            std::path::Path::to_path_buf,
        );
    let mut tmp = tempfile::NamedTempFile::new_in(&parent).map_err(|e| {
        IpcError::new("http", "download", format!("tempfile in {}: {e}", parent.display()))
            .into_string()
    })?;
    let mut reader = resp.body_mut().as_reader().take(MAX_DOWNLOAD);
    let mut buf = vec![0u8; 64 * 1024];
    let mut written: u64 = 0;
    let mut emitter = ProgressEmitter::new(progress_id, "download", total);
    loop {
        let n = reader
            .read(&mut buf)
            .map_err(|e| IpcError::new("http", "download", format!("read: {e}")).into_string())?;
        if n == 0 {
            emitter.advance(0, true);
            break;
        }
        tmp.write_all(&buf[..n])
            .map_err(|e| IpcError::new("http", "download", format!("write: {e}")).into_string())?;
        written += n as u64;
        emitter.advance(n, false);
    }
    tmp.as_file_mut()
        .flush()
        .map_err(|e| IpcError::new("http", "download", format!("flush: {e}")).into_string())?;
    tmp.persist(dest).map_err(|e| {
        IpcError::new("http", "download", format!("persist -> {dest}: {e}")).into_string()
    })?;

    Ok(json!({ "path": dest, "bytes": written }))
}

// ─────────────────────────────────────────────────────────────────────────────
// Multipart upload
//
// `ureq` doesn't ship a multipart builder; `multer` is a *parser* (server
// side). For the client side we hand-roll the framing — a few preamble
// bytes per part, a CRLF-delimited boundary, no nesting. This stays well
// inside RFC 7578 ("multipart/form-data") and lets us stream the file
// payload through a `Read` impl that pumps progress events as it goes,
// matching the request/download semantics above.
// ─────────────────────────────────────────────────────────────────────────────

/// Boundary that's vanishingly unlikely to appear in payload data:
/// 26 random hex chars after a fixed prefix. Generated per-call so
/// concurrent uploads don't share state.
///
/// Uses `faster-hex`'s SIMD-accelerated `hex_string` (same encoder
/// the `os::compute::hex_encode` hot path uses) — drops a hand-rolled
/// `for b in rng_bytes { write!("{b:02x}") }` loop in favour of the
/// crate the workspace already depends on for hash digest hex output.
fn make_boundary() -> String {
    let mut rng_bytes = [0u8; 13];
    // `getrandom` would be ideal but `rand` is already in the workspace.
    use rand::RngCore as _;
    rand::thread_rng().fill_bytes(&mut rng_bytes);
    let mut s = String::with_capacity(20 + rng_bytes.len() * 2);
    s.push_str("----TyndFormBoundary");
    s.push_str(&faster_hex::hex_string(&rng_bytes));
    s
}

/// Streaming multipart body: emits the preamble, the file contents
/// (read in 64 KB chunks), and the trailing boundary as one continuous
/// `Read`. Tracks bytes pumped so the progress emitter can report them
/// per `read()` call without doing its own bookkeeping.
struct MultipartBody {
    preamble: std::io::Cursor<Vec<u8>>,
    file: Option<fs::File>,
    epilogue: std::io::Cursor<Vec<u8>>,
    emitter: ProgressEmitter,
}

impl Read for MultipartBody {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        // Phase 1: preamble (Content-Disposition headers + CRLF separator).
        let n = self.preamble.read(buf)?;
        if n > 0 {
            self.emitter.advance(n, false);
            return Ok(n);
        }
        // Phase 2: file contents, drained in caller-sized chunks.
        if let Some(f) = self.file.as_mut() {
            let n = f.read(buf)?;
            if n > 0 {
                self.emitter.advance(n, false);
                return Ok(n);
            }
            // EOF on the file — drop it so the next read serves the trailer.
            self.file = None;
        }
        // Phase 3: trailing boundary line.
        let n = self.epilogue.read(buf)?;
        if n > 0 {
            self.emitter.advance(n, false);
        } else {
            // True EOF — flush a final progress tick so listeners see 100%.
            self.emitter.advance(0, true);
        }
        Ok(n)
    }
}

fn upload(args: &Value) -> Result<Value, String> {
    let a: UploadArgs = serde_json::from_value(args.clone()).map_err(|e| {
        IpcError::new("http", "upload", format!("invalid arguments: {e}")).into_string()
    })?;
    let url = a.url.as_str();
    super::security::check_http(url)?;
    let file_path = a.file_path.as_str();
    super::security::check_fs(file_path)?;
    let field_name = a.field_name.as_deref().unwrap_or("file");
    let timeout_ms = a.timeout_ms;
    let progress_id = a.progress_id;

    // File metadata first — total length is needed for the progress
    // emitter and (more importantly) the Content-Length header so the
    // server can pre-allocate buffers / reject oversize uploads early.
    let file = fs::File::open(file_path).map_err(|e| {
        IpcError::new("http", "upload", format!("open {file_path}: {e}")).into_string()
    })?;
    let file_len = file
        .metadata()
        .map_err(|e| {
            IpcError::new("http", "upload", format!("stat {file_path}: {e}")).into_string()
        })?
        .len();

    let filename = std::path::Path::new(file_path)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".to_string());
    let content_type = mime_guess::from_path(file_path)
        .first_or_octet_stream()
        .to_string();

    let boundary = make_boundary();
    let preamble = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"{field_name}\"; \
         filename=\"{filename}\"\r\nContent-Type: {content_type}\r\n\r\n"
    );
    let epilogue = format!("\r\n--{boundary}--\r\n");
    let total_len = preamble.len() as u64 + file_len + epilogue.len() as u64;

    let agent = build_agent(timeout_ms);
    let mut req_builder = agent.post(url);
    if let Some(obj) = a.headers.as_ref().and_then(Value::as_object) {
        for (k, v) in obj {
            if let Some(s) = v.as_str() {
                req_builder = req_builder.header(k, s);
            }
        }
    }
    // Content-Type wins over any caller-supplied value — multipart needs
    // the boundary parameter to match the body bytes exactly. Same for
    // Content-Length: ureq will otherwise switch to chunked encoding,
    // which some upload endpoints (S3 PUT, signed URLs) reject.
    req_builder = req_builder
        .header(
            "content-type",
            &format!("multipart/form-data; boundary={boundary}"),
        )
        .header("content-length", &total_len.to_string());

    let mut body = MultipartBody {
        preamble: std::io::Cursor::new(preamble.into_bytes()),
        file: Some(file),
        epilogue: std::io::Cursor::new(epilogue.into_bytes()),
        emitter: ProgressEmitter::new(progress_id, "upload", Some(total_len)),
    };

    let mut resp = req_builder
        .send(ureq::SendBody::from_reader(&mut body))
        .map_err(|e| IpcError::new("http", "upload", format!("({url}): {e}")).into_string())?;

    let status = resp.status().as_u16();
    let mut buf = Vec::new();
    resp.body_mut()
        .as_reader()
        .take(100 * 1024 * 1024)
        .read_to_end(&mut buf)
        .map_err(|e| IpcError::new("http", "upload", format!("read response: {e}")).into_string())?;

    Ok(json!({
        "status": status,
        "body": STANDARD.encode(&buf),
    }))
}

// ─── Cookie persistence — `http-cookies` feature ──────────────────────────
//
// ureq 3 exposes the in-process cookie jar via `Agent::cookie_jar_lock()`
// + `CookieJar::save_json` / `load_json`. We persist to
// `~/.tynd/cookies/<app-id>.json` (relative to `dirs::data_dir()`) so a
// cookie set on one process startup survives the next.
//
// The wire-up is opt-in: callers must (1) compile with
// `--features http-cookies` and (2) set `http.cookies = { persist:
// true, jar?: "<custom-name>" }` in `tynd.config.ts`. Default is OFF
// for back-compat — the previous build silently dropped cookies and we
// don't want to surprise apps that rely on that.

#[cfg(feature = "http-cookies")]
pub mod cookies {
    use std::path::PathBuf;

    /// Resolve the on-disk cookie jar path for `app_id`. Falls back to
    /// the system temp dir if `dirs::data_dir()` is unavailable (rare —
    /// only on environments with no `$HOME` / `%APPDATA%`).
    pub fn jar_path(app_id: &str) -> PathBuf {
        let base = dirs::data_dir()
            .unwrap_or_else(std::env::temp_dir)
            .join("tynd")
            .join("cookies");
        let _ = std::fs::create_dir_all(&base);
        base.join(format!("{app_id}.json"))
    }

    /// Load cookies from `path` into `agent`'s jar. No-op if the file
    /// doesn't exist (first startup). Errors are surfaced so callers
    /// can log them — a corrupt jar shouldn't crash the app, but
    /// silently swallowing the load is the wrong default.
    ///
    /// ureq 3's `CookieJar::load_json` requires `BufRead`, so the file
    /// goes through a `BufReader` rather than directly.
    pub fn load_into_agent(
        agent: &ureq::Agent,
        path: &std::path::Path,
    ) -> std::io::Result<()> {
        if !path.exists() {
            return Ok(());
        }
        let file = std::fs::File::open(path)?;
        let mut reader = std::io::BufReader::new(file);
        let mut jar = agent.cookie_jar_lock();
        jar.load_json(&mut reader)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
        Ok(())
    }

    /// Persist `agent`'s current jar to `path`. Atomically writes via
    /// a sibling temp file + rename so a crash mid-write can't truncate
    /// the saved cookies.
    pub fn save_from_agent(
        agent: &ureq::Agent,
        path: &std::path::Path,
    ) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut tmp = tempfile::NamedTempFile::new_in(
            path.parent().unwrap_or_else(|| std::path::Path::new(".")),
        )?;
        {
            let mut jar = agent.cookie_jar_lock();
            jar.save_json(tmp.as_file_mut())
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
        }
        tmp.persist(path)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `build_agent` should produce an agent without panicking and
    /// honour the supplied timeout. We can't exercise the network here,
    /// but the construction path is the load-bearing part of the ureq3
    /// migration — a misuse of the typestate would surface as a build
    /// error here regardless of network availability.
    #[test]
    fn build_agent_with_timeout() {
        let _agent = build_agent(Some(5_000));
    }

    #[test]
    fn build_agent_default_timeout() {
        let _agent = build_agent(None);
    }

    /// The dispatch surface should reject unknown methods cleanly.
    #[test]
    fn dispatch_rejects_unknown_method() {
        let err = dispatch("nope", &json!({})).unwrap_err();
        assert!(err.contains("unknown") || err.contains("nope"));
    }

    #[cfg(feature = "http-cookies")]
    #[test]
    fn cookie_jar_path_layout_is_predictable() {
        let p = cookies::jar_path("test-app");
        let s = p.to_string_lossy();
        assert!(s.ends_with("test-app.json"));
        assert!(s.contains("tynd"));
        assert!(s.contains("cookies"));
    }
}
