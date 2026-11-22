//! `http` OS API: `request`, `download` and `upload` over ureq, gated by the
//! URL scheme check and the `security` HTTP / filesystem allow-lists.

use std::{io::Read, time::Duration};

use serde::Deserialize;
use serde_json::Value;

use crate::api::HostApi;
use crate::ipc::error::IpcError;

use super::progress::ProgressEmitter as SharedProgressEmitter;

mod download;
mod idle;
mod request;
mod upload;

use download::download;
use request::request;
use upload::upload;

// Typed argument shapes — one `#[derive(Deserialize)]` per endpoint.
// `#[serde(default)]` keeps "missing == None" semantics for optional
// fields. Field names are camelCase, matching the JS client.
// Bodies / headers stay as `Value` — they're forwarded as-is, so
// typing them would only force a clone.

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
    file_path: String,
    #[serde(default)]
    field_name: Option<String>,
    #[serde(default)]
    timeout_ms: Option<u64>,
    #[serde(default)]
    progress_id: Option<String>,
    #[serde(default)]
    headers: Option<Value>,
}

/// Row of the host API table (`api::HOST_APIS`).
pub(crate) const API: HostApi = HostApi::new("http", dispatch);

/// IPC entry point for the `http` API. Takes `args` by value so the typed
/// Args structs deserialize without cloning the JSON tree.
pub(crate) fn dispatch(method: &str, args: Value) -> Result<Value, String> {
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
/// to JS the same way as a 200 (the client contract's "still iterable on
/// error status" branch relies on it).
///
/// SECURITY: `max_redirects(0)` disables ureq's automatic redirect
/// follower so a hostile `Location: file:///etc/passwd` (or a server
/// in an allowlisted origin redirecting to a disallowed origin) cannot
/// smuggle past `crate::security::policy::check_http(...)`, which only ran on
/// the original URL. Callers that need redirect support MUST follow
/// them manually and re-validate every hop's `Location` URL through
/// `check_http`. The 30x response surfaces to JS the same way a 200
/// would — `headers["location"]` carries the next hop, the body is
/// the server's redirect HTML.
pub(super) fn build_agent(timeout_ms: Option<u64>) -> ureq::Agent {
    let timeout = timeout_ms.map_or(Duration::from_secs(60), Duration::from_millis);
    ureq::Agent::config_builder()
        .http_status_as_error(false)
        .max_redirects(0)
        .timeout_global(Some(timeout))
        .build()
        .new_agent()
}

/// Longest a download may go without receiving a single byte.
const DOWNLOAD_IDLE_TIMEOUT: Duration = Duration::from_secs(60);

/// Agent for `http.download`. A download of any size must not hit an implicit
/// whole-call deadline, so without a caller `timeoutMs` there is no overall
/// deadline: the body streams for as long as the server keeps sending. A
/// stalled connection (no bytes for `DOWNLOAD_IDLE_TIMEOUT`) still fails, so a
/// half-open TCP connection cannot block the worker for ever.
pub(super) fn build_download_agent(timeout_ms: Option<u64>) -> ureq::Agent {
    let builder = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .max_redirects(0);
    let builder = match timeout_ms {
        Some(ms) => builder.timeout_global(Some(Duration::from_millis(ms))),
        None => builder.timeout_connect(Some(Duration::from_secs(30))),
    };
    idle::apply(builder, DOWNLOAD_IDLE_TIMEOUT)
        .build()
        .new_agent()
}

/// Reject any URL whose scheme is not `http(s)://` or `ws(s)://` before
/// the request reaches ureq. Pairs with `max_redirects(0)` so a
/// malicious `Location: file:///…` from a redirect can't slip through
/// either: the first hop is checked here, and there is no second hop
/// without an explicit caller-side re-validation.
pub(super) fn check_url_scheme(url: &str, op: &str) -> Result<(), String> {
    let parsed = url::Url::parse(url).map_err(|e| {
        IpcError::new("http", op, format!("invalid URL '{url}': {e}")).into_string()
    })?;
    match parsed.scheme() {
        "http" | "https" | "ws" | "wss" => Ok(()),
        other => Err(IpcError::new(
            "http",
            op,
            format!("scheme '{other}' is not allowed (only http/https/ws/wss)"),
        )
        .into_string()),
    }
}

/// Caller-supplied `headers` object as `(name, value)` pairs. Non-string
/// values are skipped.
pub(super) fn string_headers(headers: Option<&Value>) -> impl Iterator<Item = (&str, &str)> {
    headers
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .filter_map(|(name, value)| value.as_str().map(|value| (name.as_str(), value)))
}

/// Throttled to ≥50ms between emits so fast streams don't flood IPC.
pub(super) struct ProgressEmitter(SharedProgressEmitter);

impl ProgressEmitter {
    pub(super) fn new(id: Option<String>, phase: &'static str, total: Option<u64>) -> Self {
        Self(SharedProgressEmitter::new(
            "http:progress",
            id,
            phase,
            total,
            false,
        ))
    }

    pub(super) fn advance(&mut self, delta: usize, force: bool) {
        self.0.advance(delta, force);
    }
}

pub(super) struct ProgressReader {
    pub(super) inner: std::io::Cursor<Vec<u8>>,
    pub(super) emitter: ProgressEmitter,
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

/// Read a header value as a string. `HeaderValue::to_str` errors on
/// non-visible-ASCII bytes; such headers are treated as absent (they are
/// only ever surfaced to JS as JSON strings).
pub(super) fn header_str<'a>(
    resp: &'a ureq::http::Response<ureq::Body>,
    name: &str,
) -> Option<&'a str> {
    resp.headers().get(name).and_then(|v| v.to_str().ok())
}

/// Cap on caller-supplied request bodies. Mirrors the 100 MiB ceiling
/// the response reader enforces — symmetric so neither direction is the
/// surprise OOM vector. Anything past this is almost certainly a
/// runaway loop or a misuse (deployers wanting larger uploads should
/// route through `http.upload` which streams off disk).
pub(super) const MAX_REQUEST_BODY: usize = 100 * 1024 * 1024;
pub(super) const MAX_RESPONSE_BODY: usize = 100 * 1024 * 1024;

#[cfg(test)]
mod tests;
