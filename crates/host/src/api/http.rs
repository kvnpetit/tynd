//! `http` OS API: `request`, `stream`, `download` and `upload` over ureq,
//! gated by the URL scheme check and the `security` HTTP / filesystem
//! allow-lists (re-checked on every redirect hop, see `transport`).

use std::time::Duration;

use serde::Deserialize;
use serde_json::Value;

use crate::api::HostApi;
use crate::ipc::error::IpcError;

use super::progress::ProgressEmitter as SharedProgressEmitter;

mod download;
mod idle;
mod request;
mod transport;
mod upload;

use download::download;
use request::{request, stream};
use transport::NetArgs;
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
    #[serde(flatten)]
    net: NetArgs,
    #[serde(default)]
    progress_id: Option<String>,
    /// `http.stream` only: id carried by the `http:stream` events.
    #[serde(default)]
    stream_id: Option<String>,
    /// `http.stream` only: longest wait for the next body byte.
    #[serde(default)]
    idle_timeout_ms: Option<u64>,
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
    #[serde(flatten)]
    net: NetArgs,
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
        "stream" => stream(args),
        "download" => download(args),
        "upload" => upload(args),
        "abort" => abort(&args),
        "clearCookies" => clear_cookies(&args),
        _ => Err(IpcError::unknown_method("http", method).into_string()),
    }
}

/// `http.abort { id }`: abort the call started with `abortId: id`.
fn abort(args: &Value) -> Result<Value, String> {
    let id = args
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| IpcError::new("http", "abort", "missing string 'id'").into_string())?;
    transport::abort(id).map(Value::Bool)
}

/// `http.clearCookies { jar? }`: empty one cookie jar, or all of them.
fn clear_cookies(args: &Value) -> Result<Value, String> {
    match args.get("jar") {
        None | Some(Value::Null) => transport::clear_cookies(None),
        Some(Value::String(jar)) => {
            transport::validate_jar_name(jar, "clearCookies")?;
            transport::clear_cookies(Some(jar));
        },
        Some(_) => {
            return Err(IpcError::new("http", "clearCookies", "jar must be a string").into_string())
        },
    }
    Ok(Value::Null)
}

/// Default overall deadline of `request` / `stream` / `upload`.
pub(super) const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60);

/// ureq agent for `http.upload`: overall deadline, no redirect following
/// (a 3xx is returned; replaying a streamed file is not supported).
pub(super) fn build_agent(timeout_ms: Option<u64>) -> ureq::Agent {
    let timeout = timeout_ms.map_or(DEFAULT_TIMEOUT, Duration::from_millis);
    ureq::Agent::config_builder()
        .http_status_as_error(false)
        .max_redirects(0)
        .timeout_global(Some(timeout))
        .build()
        .new_agent()
}

/// Default `idleTimeoutMs` of `http.stream`: longest a stream may go
/// without receiving a single byte (long enough for server-sent-event
/// heartbeats).
pub(super) const DEFAULT_STREAM_IDLE_TIMEOUT: Duration = Duration::from_secs(300);

/// Longest a download may go without receiving a single byte.
pub(super) const DOWNLOAD_IDLE_TIMEOUT: Duration = Duration::from_secs(60);

/// Connect deadline of a call without any overall deadline (`download`,
/// `stream`) and without `connectTimeoutMs`.
pub(super) const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

/// Reject any URL whose scheme is not `http(s)://` or `ws(s)://` before
/// the request reaches ureq. Redirect hops go through
/// `transport::check_redirect_target` instead (http/https only).
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
