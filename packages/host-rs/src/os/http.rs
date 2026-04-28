use std::{
    fs,
    io::{Read, Write},
    time::{Duration, Instant},
};

use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::{json, Value};

use super::events;

pub fn dispatch(method: &str, args: &Value) -> Result<Value, String> {
    match method {
        "request" => request(args),
        "download" => download(args),
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
    let url = args
        .get("url")
        .and_then(Value::as_str)
        .ok_or_else(|| "http.request: missing 'url'".to_string())?;
    super::security::check_http(url)?;
    let method = args.get("method").and_then(Value::as_str).unwrap_or("GET");
    let timeout_ms = args.get("timeoutMs").and_then(Value::as_u64);
    let progress_id = args
        .get("progressId")
        .and_then(Value::as_str)
        .map(String::from);

    let agent = build_agent(timeout_ms);
    let req = apply_headers(agent.request(method, url), args.get("headers"));

    let response = if let Some(body) = args.get("body") {
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

    let response_type = args
        .get("responseType")
        .and_then(Value::as_str)
        .unwrap_or("text");

    let mut buf = Vec::new();
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
    let url = args
        .get("url")
        .and_then(Value::as_str)
        .ok_or_else(|| "http.download: missing 'url'".to_string())?;
    super::security::check_http(url)?;
    let dest = args
        .get("dest")
        .and_then(Value::as_str)
        .ok_or_else(|| "http.download: missing 'dest'".to_string())?;
    super::security::check_fs(dest)?;
    let timeout_ms = args.get("timeoutMs").and_then(Value::as_u64);
    let progress_id = args
        .get("progressId")
        .and_then(Value::as_str)
        .map(String::from);

    let agent = build_agent(timeout_ms);
    let req = apply_headers(agent.get(url), args.get("headers"));
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
