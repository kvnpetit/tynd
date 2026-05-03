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
        _ => Err(format!("http.{method}: unknown method")),
    }
}

fn build_agent(timeout_ms: Option<u64>) -> ureq::Agent {
    let mut builder = ureq::AgentBuilder::new();
    builder = if let Some(ms) = timeout_ms {
        builder.timeout(Duration::from_millis(ms))
    } else {
        builder.timeout(Duration::from_secs(60))
    };
    builder.build()
}

fn apply_headers(mut req: ureq::Request, headers: Option<&Value>) -> ureq::Request {
    if let Some(obj) = headers.and_then(Value::as_object) {
        for (k, v) in obj {
            if let Some(s) = v.as_str() {
                req = req.set(k, s);
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

fn request(args: &Value) -> Result<Value, String> {
    let a: RequestArgs = serde_json::from_value(args.clone())
        .map_err(|e| format!("http.request: invalid arguments: {e}"))?;
    let url = a.url.as_str();
    super::security::check_http(url)?;
    let method = a.method.as_deref().unwrap_or("GET");
    let timeout_ms = a.timeout_ms;
    let progress_id = a.progress_id.clone();

    let agent = build_agent(timeout_ms);
    let req = apply_headers(agent.request(method, url), a.headers.as_ref());

    let response = if let Some(body) = a.body.as_ref() {
        let (req, body_bytes) = match body {
            Value::String(s) => (req, s.as_bytes().to_vec()),
            other => {
                let json_body = serde_json::to_vec(other).map_err(|e| e.to_string())?;
                let r = if req
                    .header_names()
                    .iter()
                    .any(|h| h.eq_ignore_ascii_case("content-type"))
                {
                    req
                } else {
                    req.set("content-type", "application/json")
                };
                (r, json_body)
            },
        };
        if progress_id.is_some() {
            let total = Some(body_bytes.len() as u64);
            let reader = ProgressReader {
                inner: std::io::Cursor::new(body_bytes),
                emitter: ProgressEmitter::new(progress_id.clone(), "upload", total),
            };
            req.send(reader)
        } else {
            req.send_bytes(&body_bytes)
        }
    } else {
        req.call()
    };

    let resp = response.map_err(|e| format!("http.request({url}): {e}"))?;
    let status = resp.status();
    let status_text = resp.status_text().to_string();
    let total: Option<u64> = resp
        .header("content-length")
        .and_then(|s| s.parse::<u64>().ok());
    let mut headers_map = serde_json::Map::new();
    for name in resp.headers_names() {
        if let Some(v) = resp.header(&name) {
            headers_map.insert(name.to_lowercase(), Value::String(v.to_string()));
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
    let mut reader = resp.into_reader().take(100 * 1024 * 1024);
    let mut emitter = ProgressEmitter::new(progress_id, "download", total);
    let mut chunk = vec![0u8; 64 * 1024];
    loop {
        let n = reader
            .read(&mut chunk)
            .map_err(|e| format!("http.request: read body: {e}"))?;
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
    let a: DownloadArgs = serde_json::from_value(args.clone())
        .map_err(|e| format!("http.download: invalid arguments: {e}"))?;
    let url = a.url.as_str();
    super::security::check_http(url)?;
    let dest = a.dest.as_str();
    super::security::check_fs(dest)?;
    let timeout_ms = a.timeout_ms;
    let progress_id = a.progress_id;

    let agent = build_agent(timeout_ms);
    let req = apply_headers(agent.get(url), a.headers.as_ref());
    let resp = req
        .call()
        .map_err(|e| format!("http.download({url}): {e}"))?;

    let total: Option<u64> = resp
        .header("content-length")
        .and_then(|s| s.parse::<u64>().ok());

    if let Some(parent) = std::path::Path::new(dest).parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("http.download: mkdir {}: {e}", parent.display()))?;
    }

    // 10 GiB cap — large enough for any reasonable update binary while still
    // bounding pathological / hostile responses (servers without
    // content-length).
    const MAX_DOWNLOAD: u64 = 10 * 1024 * 1024 * 1024;
    if let Some(t) = total {
        if t > MAX_DOWNLOAD {
            return Err(format!(
                "http.download: response size {t} exceeds the {MAX_DOWNLOAD}-byte cap"
            ));
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
    let mut tmp = tempfile::NamedTempFile::new_in(&parent)
        .map_err(|e| format!("http.download: tempfile in {}: {e}", parent.display()))?;
    let mut reader = resp.into_reader().take(MAX_DOWNLOAD);
    let mut buf = vec![0u8; 64 * 1024];
    let mut written: u64 = 0;
    let mut emitter = ProgressEmitter::new(progress_id, "download", total);
    loop {
        let n = reader
            .read(&mut buf)
            .map_err(|e| format!("http.download: read: {e}"))?;
        if n == 0 {
            emitter.advance(0, true);
            break;
        }
        tmp.write_all(&buf[..n])
            .map_err(|e| format!("http.download: write: {e}"))?;
        written += n as u64;
        emitter.advance(n, false);
    }
    tmp.as_file_mut()
        .flush()
        .map_err(|e| format!("http.download: flush: {e}"))?;
    tmp.persist(dest)
        .map_err(|e| format!("http.download: persist -> {dest}: {e}"))?;

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
fn make_boundary() -> String {
    use std::fmt::Write as _;
    let mut s = String::from("----TyndFormBoundary");
    let mut rng_bytes = [0u8; 13];
    // `getrandom` would be ideal but `rand` is already in the workspace.
    use rand::RngCore as _;
    rand::thread_rng().fill_bytes(&mut rng_bytes);
    for b in rng_bytes {
        let _ = write!(&mut s, "{b:02x}");
    }
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
    let a: UploadArgs = serde_json::from_value(args.clone())
        .map_err(|e| format!("http.upload: invalid arguments: {e}"))?;
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
    let file = fs::File::open(file_path)
        .map_err(|e| format!("http.upload: open {file_path}: {e}"))?;
    let file_len = file
        .metadata()
        .map_err(|e| format!("http.upload: stat {file_path}: {e}"))?
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
    let mut req = agent.post(url);
    req = apply_headers(req, a.headers.as_ref());
    // Content-Type wins over any caller-supplied value — multipart needs
    // the boundary parameter to match the body bytes exactly. Same for
    // Content-Length: ureq will otherwise switch to chunked encoding,
    // which some upload endpoints (S3 PUT, signed URLs) reject.
    req = req
        .set(
            "content-type",
            &format!("multipart/form-data; boundary={boundary}"),
        )
        .set("content-length", &total_len.to_string());

    let body = MultipartBody {
        preamble: std::io::Cursor::new(preamble.into_bytes()),
        file: Some(file),
        epilogue: std::io::Cursor::new(epilogue.into_bytes()),
        emitter: ProgressEmitter::new(progress_id, "upload", Some(total_len)),
    };

    let resp = req
        .send(body)
        .map_err(|e| format!("http.upload({url}): {e}"))?;

    let status = resp.status();
    let mut buf = Vec::new();
    resp.into_reader()
        .take(100 * 1024 * 1024)
        .read_to_end(&mut buf)
        .map_err(|e| format!("http.upload: read response: {e}"))?;

    Ok(json!({
        "status": status,
        "body": STANDARD.encode(&buf),
    }))
}
