//! Auto-updater — signed manifest check + signed binary download.
//!
//! **Trust model.** The manifest bytes are verified with an Ed25519 signature
//! fetched from `<endpoint>.sig` (or `manifestSignatureUrl`) before parsing.
//! Each platform entry then carries a second Ed25519 signature over the raw
//! bytes of the downloaded file, so both manifest metadata and payloads are
//! covered by the pub-key baked into the app binary.
//!
//! **What the caller cannot choose.** The version floor and the manifest
//! endpoints never come from IPC arguments: `check` compares against the app's
//! own version (`app.setInfo`, set by the backend at startup) and fetches only
//! the endpoints of the host-trusted updater configuration (`updater.configure`,
//! backend/plugin only, or `TYND_FRONTEND_RUNTIME_CONFIG` for CLI-launched
//! runs). Caller-supplied `currentVersion`, `endpoints` and
//! `manifestSignatureUrl` are ignored, so a compromised renderer cannot replay
//! an old, validly signed manifest. The only caller-influenced selector is the
//! release channel, through the persisted `setChannel` override.
//!
//! **Private servers.** The backend-only `updater.configure` config may also
//! carry `proxy` (an http/https proxy URL) and `headers` (fixed request
//! headers, e.g. `Authorization`). Headers go only to the origins of the
//! configured endpoints and never follow a cross-origin redirect; renderer
//! supplied `proxy` / `headers` are ignored.
//!
//! **Distribution.** `endpoints` are tried in order (next mirror after a
//! network error or HTTP 5xx/408/429) and may use the `{{channel}}`,
//! `{{target}}`, `{{arch}}` and `{{current_version}}` placeholders. The channel
//! comes from the persisted `setChannel` override, else the configured
//! default. Signatures verify against any of the trusted keys (key rotation).
//! A manifest may carry `channel` (checked against the install's explicit
//! channel) and `rollout` (0..=100; each install has a stable random bucket).
//!
//! Manifest format (Tauri-compatible enough for cross-port):
//! ```json
//! {
//!   "version": "1.2.3",
//!   "notes": "release notes...",
//!   "pub_date": "2026-04-19T12:00:00Z",
//!   "channel": "stable",
//!   "rollout": 100,
//!   "platforms": {
//!     "windows-x86_64": { "url": "...", "signature": "<base64 Ed25519>" },
//!     "darwin-aarch64": { "url": "...", "signature": "<base64 Ed25519>" }
//!   }
//! }
//! ```

use std::{
    collections::HashSet,
    fs,
    io::{Read, Write},
    path::PathBuf,
    sync::LazyLock,
    time::Duration,
};

#[cfg(target_os = "macos")]
use std::sync::atomic::{AtomicU64, Ordering};

use base64::{engine::general_purpose::STANDARD, Engine};
// SECURITY: BLAKE3 fingerprint of every downloaded + signature-verified
// binary. Returned by `download_and_verify` in the JSON IPC payload and
// re-checked by `install` immediately before `self_replace`. The window
// between download and install is the TOCTOU surface — without this
// re-check, an attacker who can write to `temp_dir` between the two IPC
// calls swaps the binary for one that wasn't covered by the Ed25519
// verification.
// Hex-encoded (32 bytes → 64 hex chars) so it survives a JSON round-trip
// without base64 framing surprises and reads as the same string a user
// would compute via `b3sum`.
use blake3::Hasher as Blake3Hasher;
use ed25519_dalek::{Signature, VerifyingKey, PUBLIC_KEY_LENGTH, SIGNATURE_LENGTH};
use parking_lot::Mutex;
use serde_json::{json, Value};

use crate::api::HostApi;
use crate::ipc::error::IpcError;

use super::progress::ProgressEmitter;
use crate::ipc::events;

mod check;
mod download;
mod install;
mod network;
mod patch;
mod periodic;
mod state;

#[derive(Clone)]
struct AuthorizedDownload {
    url: String,
    signature: String,
    format: String,
    release_signature: Option<String>,
}

static AUTHORIZED_DOWNLOADS: LazyLock<Mutex<Vec<AuthorizedDownload>>> =
    LazyLock::new(|| Mutex::new(Vec::new()));

fn authorize_manifest_downloads(
    url: &str,
    signature: &str,
    format: &str,
    patch: Option<&Value>,
) -> Result<(), String> {
    let mut authorized = vec![AuthorizedDownload {
        url: url.to_string(),
        signature: signature.to_string(),
        format: format.to_string(),
        release_signature: None,
    }];
    if let Some(patch) = patch {
        let patch_url = patch
            .get("url")
            .and_then(Value::as_str)
            .ok_or("updater.check: patch missing 'url'")?;
        let patch_signature = patch
            .get("signature")
            .and_then(Value::as_str)
            .ok_or("updater.check: patch missing 'signature'")?;
        authorized.push(AuthorizedDownload {
            url: patch_url.to_string(),
            signature: patch_signature.to_string(),
            format: "binary".to_string(),
            release_signature: Some(signature.to_string()),
        });
    }
    *AUTHORIZED_DOWNLOADS.lock() = authorized;
    Ok(())
}

fn require_authorized_download(
    url: &str,
    signature: &str,
    format: &str,
    release_signature: Option<&str>,
) -> Result<(), String> {
    let allowed = AUTHORIZED_DOWNLOADS.lock().iter().any(|entry| {
        entry.url == url
            && entry.signature == signature
            && entry.format == format
            && entry.release_signature.as_deref() == release_signature
    });
    if allowed {
        Ok(())
    } else {
        Err(
            "updater: download is not authorized by the latest verified manifest; call check first"
                .into(),
        )
    }
}

use check::check;
use download::download_and_verify;
use install::{ensure_self_updatable, install};
use patch::download_and_patch;
use periodic::{start_periodic_check, stop_periodic_check};
use tynd_update_core::{
    first_verifying_key, is_retryable_status, split_key_list, validate_channel, FetchFailure,
};
pub(crate) use tynd_update_core::{
    hashes_match, is_update_available, version_eq, PatchCompression, UpdaterConfig,
};

const DEFAULT_HTTP_TIMEOUT: Duration = Duration::from_secs(60);
const PERIODIC_HTTP_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_HTTP_REDIRECTS: usize = 3;

/// The document of webview `label` is gone (commit or close): staged artifacts
/// it owns are re-owned to the backend.
pub(crate) fn release_webview(label: &str) {
    install::release_owner(label);
}

pub(crate) fn shutdown() {
    periodic::shutdown();
}

/// Row of the host API table (`api::HOST_APIS`).
pub(crate) const API: HostApi = HostApi {
    shutdown: Some(shutdown),
    release_webview: Some(release_webview),
    ..HostApi::new("updater", |method, args| dispatch(method, &args))
};

pub(crate) fn dispatch(method: &str, args: &Value) -> Result<Value, String> {
    match method {
        "check" => check(args),
        "downloadAndVerify" => {
            ensure_self_updatable("downloadAndVerify")?;
            download_and_verify(args)
        },
        // BSDIFF delta download: downloads a small patch (~100 KB) and
        // applies it on top of the currently-running binary, producing
        // the same staged file layout `install` consumes. Falls back to
        // `download_and_verify` semantics on the result side (path +
        // size + BLAKE3 hex), so the JS caller can route either result
        // through the same single-use handle `install` arm.
        "downloadAndPatch" => {
            ensure_self_updatable("downloadAndPatch")?;
            download_and_patch(args)
        },
        "install" => install(args),
        // Backend-only (denied for renderers and plugins in `api::dispatch_*`):
        // records the host-trusted endpoints / channel used by `check`.
        "configure" => configure(args),
        "startPeriodicCheck" => start_periodic_check(args),
        "stopPeriodicCheck" => {
            stop_periodic_check();
            Ok(Value::Null)
        },
        // Release channel. `setChannel` is validated and persisted per install;
        // `getChannel` returns the runtime override, else `args.default`, else
        // the built-in default.
        "getChannel" => {
            Ok(json!({ "channel": effective_channel(args.get("default").and_then(Value::as_str)) }))
        },
        "setChannel" => {
            let name = args
                .get("channel")
                .and_then(Value::as_str)
                .ok_or("updater.setChannel: missing 'channel'")?;
            state::set_runtime_channel(name)?;
            Ok(json!({ "channel": name }))
        },
        // Echo the user-preference block back to the JS layer. The CLI
        // injects `globalThis.__TYND_UPDATER_CONFIG__` at bundle time so
        // the JS wrapper already has the same data; this arm is kept as
        // an optional round-trip for tests and for callers that want the
        // host to be the single source of truth post-validation. Args
        // are deserialised through `UpdaterConfig` so a malformed bundle
        // surfaces a clean error here rather than the silent
        // "preference ignored" failure mode it would otherwise hit.
        "validateConfig" => validate_config(args),
        _ => Err(IpcError::unknown_method("updater", method).into_string()),
    }
}

/// Host-trusted updater configuration (endpoints, default channel). Never
/// derived from a renderer call; see the module docs.
static TRUSTED_CONFIG: LazyLock<Mutex<Option<UpdaterConfig>>> = LazyLock::new(|| Mutex::new(None));

/// Host-trusted proxy and request headers, installed together with
/// [`TRUSTED_CONFIG`].
static TRUSTED_NETWORK: LazyLock<Mutex<Option<network::TrustedNetwork>>> =
    LazyLock::new(|| Mutex::new(None));

fn parse_trusted(value: &Value) -> Result<(UpdaterConfig, network::TrustedNetwork), String> {
    let cfg = serde_json::from_value::<UpdaterConfig>(value.clone())
        .map_err(|e| format!("updater.configure: {e}"))?
        .normalised();
    cfg.validate_distribution()
        .map_err(|e| format!("updater.configure: {e}"))?;
    let net = network::TrustedNetwork::from_config(value, &cfg.resolved_endpoints())
        .map_err(|e| format!("updater.configure: {e}"))?;
    Ok((cfg, net))
}

/// The CLI-provided `TYND_FRONTEND_RUNTIME_CONFIG` (set before launch, so not
/// reachable from a renderer), parsed.
fn env_trusted() -> Option<(UpdaterConfig, network::TrustedNetwork)> {
    let raw = std::env::var("TYND_FRONTEND_RUNTIME_CONFIG").ok()?;
    parse_env_trusted(&raw)
}

/// Parse the `TYND_FRONTEND_RUNTIME_CONFIG` payload. A malformed or invalid
/// value is logged, so a misconfigured updater is not reported only as
/// "no host-trusted updater configuration".
fn parse_env_trusted(raw: &str) -> Option<(UpdaterConfig, network::TrustedNetwork)> {
    let value = match serde_json::from_str::<Value>(raw) {
        Ok(value) => value,
        Err(error) => {
            tracing::warn!(
                target: "tynd",
                "ignoring invalid TYND_FRONTEND_RUNTIME_CONFIG: {error}"
            );
            return None;
        },
    };
    if value.is_null() {
        return None;
    }
    match parse_trusted(&value) {
        Ok(trusted) => Some(trusted),
        Err(error) => {
            tracing::warn!(
                target: "tynd",
                "ignoring invalid TYND_FRONTEND_RUNTIME_CONFIG updater configuration: {error}"
            );
            None
        },
    }
}

/// `updater.configure(config)`: trusted callers (backend startup, native
/// plugins) install the updater configuration `check` relies on, including the
/// optional `proxy` and `headers` for private update servers.
fn configure(args: &Value) -> Result<Value, String> {
    let (cfg, net) = parse_trusted(args)?;
    let endpoints = cfg.resolved_endpoints();
    *TRUSTED_CONFIG.lock() = Some(cfg);
    *TRUSTED_NETWORK.lock() = Some(net);
    Ok(json!({ "ok": true, "endpoints": endpoints }))
}

/// Proxy and headers of the trusted configuration (none when unconfigured).
fn trusted_network() -> network::TrustedNetwork {
    if let Some(net) = TRUSTED_NETWORK.lock().clone() {
        return net;
    }
    env_trusted().map(|(_, net)| net).unwrap_or_default()
}

/// The configuration `check` must use: the one installed by `configure`, else
/// the CLI-provided `TYND_FRONTEND_RUNTIME_CONFIG` (set before launch, so not
/// reachable from a renderer).
pub(super) fn trusted_config() -> Result<UpdaterConfig, String> {
    if let Some(cfg) = TRUSTED_CONFIG.lock().clone() {
        return Ok(cfg);
    }
    env_trusted()
        .map(|(cfg, _)| cfg)
        .ok_or_else(|| {
            "updater: no host-trusted updater configuration - the backend must call `updater.configure(...)` at startup (endpoints are never taken from the caller)"
                .to_string()
        })
}

pub(super) fn validate_config(args: &Value) -> Result<Value, String> {
    let cfg: UpdaterConfig =
        serde_json::from_value(args.clone()).map_err(|e| format!("updater.validateConfig: {e}"))?;
    let cfg = cfg.normalised();
    cfg.validate_distribution()
        .map_err(|e| format!("updater.validateConfig: {e}"))?;
    Ok(json!({
        "ok": true,
        "endpoints": cfg.resolved_endpoints(),
        "channel": cfg.channel,
        "checkIntervalSecs": cfg.check_interval_secs,
        "preferPatch": cfg.prefer_patch,
        "allowPatch": cfg.allow_patch,
        "maxPatchSize": cfg.max_patch_size,
        "fallbackToFull": cfg.fallback_to_full,
        "autoCheck": cfg.auto_check,
        "checkOnStartup": cfg.check_on_startup,
        "autoDownload": cfg.auto_download,
        "autoApply": cfg.auto_apply,
        "promptStyle": cfg.prompt_style.as_ref(),
        "requireSignature": cfg.require_signature,
        "requireMajorPrompt": cfg.require_major_prompt,
    }))
}

/// Strip query string and userinfo from a URL so error messages, tracing
/// events, and Sentry breadcrumbs don't leak `?token=`, `?access_key=`,
/// or `https://user:pass@host` credentials. Falls back to the bare host
/// when parsing fails so we never accidentally re-emit the raw input.
pub(super) fn redact_url(url: &str) -> String {
    match url::Url::parse(url) {
        Ok(mut p) => {
            p.set_query(None);
            let _ = p.set_username("");
            let _ = p.set_password(None);
            p.set_fragment(None);
            p.to_string()
        },
        Err(_) => "<unparseable url>".to_string(),
    }
}

pub(super) fn verify_http_url(url: &str) -> Result<(), String> {
    let parsed = url::Url::parse(url).map_err(|e| format!("updater: invalid URL: {e}"))?;
    match parsed.scheme() {
        "https" => crate::security::policy::check_http(parsed.as_str()),
        "http" if is_loopback_url(&parsed) => {
            crate::security::policy::check_http_configured(parsed.as_str()).map_err(|policy_err| {
                format!(
                    "updater: loopback HTTP URL '{}' requires an explicit HTTP security policy: \
                     {policy_err}",
                    redact_url(parsed.as_str())
                )
            })
        },
        scheme => Err(format!(
            "updater: refusing URL '{}' with scheme '{scheme}'; updater traffic requires HTTPS",
            redact_url(parsed.as_str())
        )),
    }
}

fn is_loopback_url(url: &url::Url) -> bool {
    let Some(host) = url.host_str() else {
        return false;
    };
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}

/// HTTP agent for updater traffic. The proxy is never caller-selected: it is
/// the one of the trusted config (`updater.configure`), else ureq applies the
/// process environment (`HTTPS_PROXY` / `ALL_PROXY`).
pub(super) fn http_agent() -> Result<ureq::Agent, String> {
    http_agent_with_timeout(DEFAULT_HTTP_TIMEOUT)
}

pub(super) fn http_agent_with_timeout(timeout: Duration) -> Result<ureq::Agent, String> {
    let config = ureq::Agent::config_builder()
        .http_status_as_error(false)
        // Redirects are followed only by `get_with_redirects`, which checks
        // every resolved Location against the updater + deployer policies.
        .max_redirects(0)
        .timeout_global(Some(timeout));
    let config = match trusted_network().proxy() {
        Some(proxy) => config.proxy(Some(
            ureq::Proxy::new(proxy).map_err(|_| "updater: the configured proxy is invalid")?,
        )),
        None => config,
    };

    Ok(config.build().new_agent())
}

/// Execute an updater GET while validating every redirect destination.
///
/// Relative `Location` values are resolved against the current URL, loops and
/// HTTPS downgrades are rejected, and at most `MAX_HTTP_REDIRECTS` hops are
/// followed. Cross-origin redirects remain supported for signed release CDN
/// URLs. No caller-provided headers are ever sent; the headers of the trusted
/// config go only to the origins of the configured endpoints.
pub(super) fn get_with_redirects(
    agent: &ureq::Agent,
    url: &str,
    operation: &str,
) -> Result<ureq::http::Response<ureq::Body>, String> {
    get_with_redirects_classified(agent, url, operation).map_err(FetchFailure::into_message)
}

/// Like [`get_with_redirects`], but tells transport failures (worth retrying
/// on another mirror) apart from policy refusals (never retried). A response
/// with a retryable HTTP status (5xx, 408, 429) is also reported as retryable;
/// other statuses are returned to the caller unchanged.
pub(super) fn get_with_redirects_classified(
    agent: &ureq::Agent,
    url: &str,
    operation: &str,
) -> Result<ureq::http::Response<ureq::Body>, FetchFailure> {
    get_with_redirects_inner(agent, url, operation).and_then(|response| {
        let status = response.status().as_u16();
        if is_retryable_status(status) {
            Err(FetchFailure::Retryable(format!(
                "{operation}({}): HTTP {status}",
                redact_url(url)
            )))
        } else {
            Ok(response)
        }
    })
}

fn get_with_redirects_inner(
    agent: &ureq::Agent,
    url: &str,
    operation: &str,
) -> Result<ureq::http::Response<ureq::Body>, FetchFailure> {
    let fatal = |message: String| FetchFailure::Fatal(message);
    let mut current = url::Url::parse(url).map_err(|error| {
        fatal(format!(
            "{operation}: invalid updater URL '{}': {error}",
            redact_url(url)
        ))
    })?;
    current.set_fragment(None);
    verify_http_url(current.as_str()).map_err(fatal)?;

    let mut visited = HashSet::with_capacity(MAX_HTTP_REDIRECTS + 1);
    visited.insert(current.as_str().to_string());
    let network = trusted_network();

    for redirects_followed in 0..=MAX_HTTP_REDIRECTS {
        let mut request = agent.get(current.as_str());
        for (name, value) in network.headers_for(&current) {
            request = request.header(name.clone(), value.clone());
        }
        // Transport errors (DNS, connect, timeout, TLS) are the only retryable
        // failures raised here; every policy refusal below is fatal.
        let response = request.call().map_err(|error| {
            FetchFailure::Retryable(format!(
                "{operation}({}): {error}",
                redact_url(current.as_str())
            ))
        })?;
        let status = response.status().as_u16();
        if !matches!(status, 301 | 302 | 303 | 307 | 308) {
            return Ok(response);
        }
        if redirects_followed == MAX_HTTP_REDIRECTS {
            return Err(fatal(format!(
                "{operation}: redirect limit exceeded after {MAX_HTTP_REDIRECTS} hops at '{}'",
                redact_url(current.as_str())
            )));
        }

        let next = redirect_target(&current, &response, status, operation).map_err(fatal)?;
        verify_http_url(next.as_str())
            .map_err(|error| fatal(format!("{operation}: redirect denied: {error}")))?;
        if !visited.insert(next.as_str().to_string()) {
            return Err(fatal(format!(
                "{operation}: redirect loop detected at '{}'",
                redact_url(next.as_str())
            )));
        }
        current = next;
    }

    unreachable!("redirect loop returns at its configured limit")
}

/// Resolve the `Location` of a redirect `response` from `current` into the
/// next URL (fragment dropped). Refuses a missing, non-UTF-8 or empty header
/// and an HTTPS-to-HTTP downgrade.
fn redirect_target(
    current: &url::Url,
    response: &ureq::http::Response<ureq::Body>,
    status: u16,
    operation: &str,
) -> Result<url::Url, String> {
    let location = response
        .headers()
        .get(ureq::http::header::LOCATION)
        .ok_or_else(|| {
            format!(
                "{operation}: HTTP {status} from '{}' has no Location header",
                redact_url(current.as_str())
            )
        })?
        .to_str()
        .map_err(|_| {
            format!(
                "{operation}: HTTP {status} from '{}' has a non-UTF-8 Location header",
                redact_url(current.as_str())
            )
        })?
        .trim()
        .to_string();
    if location.is_empty() {
        return Err(format!(
            "{operation}: HTTP {status} from '{}' has an empty Location header",
            redact_url(current.as_str())
        ));
    }

    let mut next = current.join(&location).map_err(|error| {
        format!(
            "{operation}: invalid redirect Location from '{}': {error}",
            redact_url(current.as_str())
        )
    })?;
    next.set_fragment(None);
    if current.scheme() == "https" && next.scheme() == "http" {
        return Err(format!(
            "{operation}: refusing HTTPS downgrade redirect from '{}' to '{}'",
            redact_url(current.as_str()),
            redact_url(next.as_str())
        ));
    }
    Ok(next)
}

/// Stream the file at `path` through BLAKE3 and return the lowercase hex
/// digest. Used by `install` to re-verify the staged update binary
/// immediately before `self_replace` swaps it in. Streaming (rather
/// than `read` + `hash`) keeps the peak memory bounded for >100 MB
/// updates.
pub(super) fn blake3_file(path: &std::path::Path) -> std::io::Result<String> {
    let mut hasher = Blake3Hasher::new();
    let mut file = fs::File::open(path)?;
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finalize().to_hex().to_string())
}

/// Securely create and write a staged updater payload.
///
/// The caller cannot choose the path: `tempfile` creates a random file with
/// create-new semantics, closing the predictable-name/symlink race that a
/// `temp_dir().join(...); OpenOptions::truncate(true)` sequence would expose.
/// Unix permissions are forced to `0600`; Windows inherits the user's temp
/// directory ACL. The path is kept only after every byte and metadata flush
/// succeeds, so failed writes clean themselves up.
pub(super) fn write_staged_blob(bytes: &[u8]) -> std::io::Result<PathBuf> {
    let mut staged = tempfile::Builder::new().prefix("tynd-update-").tempfile()?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        staged
            .as_file()
            .set_permissions(fs::Permissions::from_mode(0o600))?;
    }

    staged.as_file_mut().write_all(bytes)?;
    staged.as_file_mut().sync_all()?;
    staged.keep().map(|(_, path)| path).map_err(|e| e.error)
}

#[cfg(target_os = "macos")]
pub(super) fn simple_rand() -> String {
    // Small, non-crypto unique suffix. PID + full nanos + atomic counter so
    // two parallel calls in the same process can't collide.
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos() as u64);
    let pid = u64::from(std::process::id());
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{pid:x}-{nanos:x}-{n:x}")
}

pub(super) fn platform_key() -> String {
    let os = match std::env::consts::OS {
        "macos" => "darwin",
        other => other,
    };
    let arch = std::env::consts::ARCH;
    format!("{os}-{arch}")
}

pub(super) fn decode_signature(b64: &str) -> Result<Signature, String> {
    let raw = STANDARD
        .decode(b64.trim())
        .map_err(|e| format!("signature base64: {e}"))?;
    if raw.len() != SIGNATURE_LENGTH {
        return Err(format!(
            "signature length: expected {SIGNATURE_LENGTH}, got {}",
            raw.len()
        ));
    }
    let bytes: [u8; SIGNATURE_LENGTH] = raw
        .as_slice()
        .try_into()
        .map_err(|_| "signature: bad length".to_string())?;
    Ok(Signature::from_bytes(&bytes))
}

/// Compile-time updater verifying key(s). Set `TYND_UPDATER_PUBKEY` to a
/// base64-encoded Ed25519 public key at build time to pin the key the
/// host accepts. For key rotation, list several keys separated by commas
/// (or whitespace / semicolons); a signature is then valid when any of them
/// verifies it.
///
/// SECURITY WARNING: this is a hard requirement, not a hint. When the
/// env var is unset or empty at build time, every `updater.*` call that
/// verifies a signature is REJECTED. Keys come only from the trusted updater
/// configuration (never from IPC arguments) and each must match one compile-time
/// pin byte-for-byte, so a tampered bundle cannot hand the verifier a key whose
/// private half is the attacker's.
const COMPILED_PUBKEY_B64: Option<&str> = option_env!("TYND_UPDATER_PUBKEY");

fn decode_pub_key_with_pin(b64: &str, compiled_pin: Option<&str>) -> Result<VerifyingKey, String> {
    // SECURITY: the configured key MUST match a compile-time pin.
    // - No pin (None / empty) → refuse every key: there is nothing to bind the runtime decision
    //   to, so accepting any key would let a malicious manifest pick its own verifier.
    // - Pin set + mismatch → refuse: same threat model.
    // - Pin set + match → accept (and decode below).
    {
        let pins = compiled_pin.map(split_key_list).unwrap_or_default();
        if pins.is_empty() {
            return Err("pubKey: refusing key — host was built without a \
                 `TYND_UPDATER_PUBKEY` pin. Rebuild the host with the pubkey baked in to enable \
                 updater calls."
                .into());
        }
        if !pins.contains(&b64.trim()) {
            return Err("pubKey: key does not match the compiled-in \
                 `TYND_UPDATER_PUBKEY`; refusing"
                .into());
        }
    }

    let encoded = b64.trim().strip_prefix("ed25519:").unwrap_or(b64.trim());
    let raw = STANDARD
        .decode(encoded)
        .map_err(|e| format!("pubKey base64: {e}"))?;
    if raw.len() != PUBLIC_KEY_LENGTH {
        return Err(format!(
            "pubKey length: expected {PUBLIC_KEY_LENGTH}, got {}",
            raw.len()
        ));
    }
    let bytes: [u8; PUBLIC_KEY_LENGTH] = raw
        .as_slice()
        .try_into()
        .map_err(|_| "pubKey: bad length".to_string())?;
    VerifyingKey::from_bytes(&bytes).map_err(|e| format!("pubKey: {e}"))
}

/// The set of trusted verifying keys for one updater call (key rotation).
pub(super) struct KeySet(Vec<VerifyingKey>);

impl KeySet {
    /// The verifying keys of the host-trusted updater configuration
    /// (`updater.configure` / CLI-provided runtime config). Caller-supplied
    /// arguments are never read: a compromised renderer must not be able to
    /// choose the key that authenticates an update. Every key must also match
    /// the compiled-in pin; a host built without one refuses every key.
    pub(super) fn from_trusted(operation: &str) -> Result<Self, String> {
        Self::from_config_with_pin(&trusted_config()?, operation, COMPILED_PUBKEY_B64)
    }

    fn from_config_with_pin(
        cfg: &UpdaterConfig,
        operation: &str,
        compiled_pin: Option<&str>,
    ) -> Result<Self, String> {
        let mut keys = Vec::new();
        for b64 in cfg.resolved_public_keys() {
            keys.push(decode_pub_key_with_pin(&b64, compiled_pin)?);
        }
        if keys.is_empty() {
            return Err(format!(
                "{operation}: the trusted updater config has no public key"
            ));
        }
        Ok(Self(keys))
    }

    /// `Ok` when any trusted key verifies `signature` over `message`.
    pub(super) fn verify(&self, message: &[u8], signature: &Signature) -> Result<(), String> {
        use ed25519_dalek::Verifier as _;
        if first_verifying_key(&self.0, |key| key.verify(message, signature).is_ok()).is_some() {
            Ok(())
        } else if self.0.len() == 1 {
            Err("signature does not match the trusted key".into())
        } else {
            Err(format!(
                "signature does not match any of the {} trusted keys",
                self.0.len()
            ))
        }
    }
}

/// Channel for this run: runtime override (persisted `setChannel`), then the
/// build-time default, else `None`.
pub(super) fn explicit_channel(default: Option<&str>) -> Option<String> {
    state::runtime_channel().or_else(|| {
        default
            .map(str::trim)
            .filter(|c| !c.is_empty() && validate_channel(c).is_ok())
            .map(str::to_string)
    })
}

fn effective_channel(default: Option<&str>) -> String {
    explicit_channel(default).unwrap_or_else(|| tynd_update_core::DEFAULT_CHANNEL.to_string())
}

/// `(target, arch)` halves of [`platform_key`] (`windows`, `x86_64`).
pub(super) fn platform_parts() -> (&'static str, &'static str) {
    let os = match std::env::consts::OS {
        "macos" => "darwin",
        other => other,
    };
    (os, std::env::consts::ARCH)
}

pub(super) fn is_strictly_newer(current: &str, available: &str) -> bool {
    // Delegate to the semver-backed public helper for accurate pre-release
    // ordering. Fall back to `true` (assume update available) on any parse
    // failure so a malformed manifest version doesn't silently block the check.
    is_update_available(current, available).unwrap_or(true)
}

pub(super) struct Progress(ProgressEmitter);

impl Progress {
    pub(super) fn new(id: Option<String>, phase: &'static str, total: Option<u64>) -> Self {
        Self(ProgressEmitter::new(
            "updater:progress",
            id,
            phase,
            total,
            true,
        ))
    }

    pub(super) fn advance(&mut self, delta: usize, force: bool) {
        self.0.advance(delta, force);
    }

    pub(super) fn emit_phase(&mut self, phase: &'static str) {
        self.0.emit_phase(phase);
    }
}

#[cfg(test)]
mod tests;
