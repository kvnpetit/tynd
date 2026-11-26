//! Transport shared by `http.request`, `http.stream` and `http.download`:
//! agent options (timeouts, proxy), the redirect follower that re-runs the
//! scheme and `security` checks on every hop, the in-memory cookie jars and
//! the abort registry.

use std::{
    fmt::Display,
    io::Read,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, LazyLock,
    },
    time::Duration,
};

use bytes::Bytes;
use cookie_store::CookieStore;
use parking_lot::Mutex;
use serde::Deserialize;
use serde_json::Value;
use url::Url;

use crate::ipc::{error::IpcError, owner::Owner};

use super::{idle, ProgressEmitter};

type Response = ureq::http::Response<ureq::Body>;

/// Default redirect cap when following (`maxRedirects` omitted).
pub(super) const DEFAULT_MAX_REDIRECTS: u32 = 10;
/// Largest `maxRedirects` a caller may ask for.
pub(super) const MAX_REDIRECTS_CAP: u32 = 50;
/// Live cookie jars per process.
const MAX_JARS: usize = 32;
/// Abort ids remembered before their request registered.
const MAX_PENDING_ABORTS: usize = 256;
/// How often a call waiting for its response head re-checks its abort flag
/// and caller.
pub(super) const HEAD_POLL: Duration = Duration::from_millis(100);

fn op_error(op: &str, msg: impl Display) -> String {
    IpcError::new("http", op, msg).into_string()
}

/// `redirect` option.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub(super) enum RedirectMode {
    /// Follow up to `maxRedirects` hops, each re-checked.
    #[default]
    Follow,
    /// Return the 3xx response as is.
    Manual,
    /// Reject on the first redirect.
    Error,
}

/// Transport options shared by every endpoint (flattened into its args).
#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct NetArgs {
    #[serde(default)]
    pub(super) timeout_ms: Option<u64>,
    #[serde(default)]
    pub(super) connect_timeout_ms: Option<u64>,
    /// URL string, `false` (no proxy) or absent (environment proxy).
    #[serde(default)]
    pub(super) proxy: Option<Value>,
    #[serde(default)]
    pub(super) redirect: Option<RedirectMode>,
    #[serde(default)]
    pub(super) max_redirects: Option<u32>,
    /// `true` (default jar), a jar name, or absent / `false` (no jar).
    #[serde(default)]
    pub(super) cookies: Option<Value>,
    #[serde(default)]
    pub(super) abort_id: Option<String>,
}

/// Proxy selection for one call.
pub(super) enum ProxyChoice {
    /// `HTTPS_PROXY` / `HTTP_PROXY` / `ALL_PROXY` / `NO_PROXY` (ureq default).
    Env,
    /// Connect directly, ignoring the environment.
    Direct,
    Url(ureq::Proxy),
}

/// Validated transport settings for one call.
pub(super) struct Net {
    pub(super) timeout: Option<Duration>,
    pub(super) connect_timeout: Option<Duration>,
    pub(super) proxy: ProxyChoice,
    pub(super) redirect: RedirectMode,
    pub(super) max_redirects: u32,
    /// Cookie jar of the calling identity, when `cookies` is on.
    pub(super) jar: Option<JarKey>,
}

impl Net {
    /// Validate `args`; `default_timeout` applies when `timeoutMs` is absent.
    pub(super) fn parse(
        args: &NetArgs,
        op: &str,
        default_timeout: Option<Duration>,
    ) -> Result<Self, String> {
        let max_redirects = args.max_redirects.unwrap_or(DEFAULT_MAX_REDIRECTS);
        if max_redirects > MAX_REDIRECTS_CAP {
            return Err(op_error(
                op,
                format!("maxRedirects must be at most {MAX_REDIRECTS_CAP}, got {max_redirects}"),
            ));
        }
        Ok(Self {
            timeout: args
                .timeout_ms
                .map(Duration::from_millis)
                .or(default_timeout),
            connect_timeout: args.connect_timeout_ms.map(Duration::from_millis),
            proxy: parse_proxy(args.proxy.as_ref(), op)?,
            redirect: args.redirect.unwrap_or_default(),
            max_redirects,
            jar: parse_jar(args.cookies.as_ref(), op)?.map(JarKey::for_caller),
        })
    }

    /// ureq agent for this call. `idle` caps each transport read (downloads).
    pub(super) fn agent(&self, idle: Option<Duration>) -> ureq::Agent {
        let mut builder = ureq::Agent::config_builder()
            .http_status_as_error(false)
            // Redirects are followed by [`execute`], never by ureq: every hop
            // must go through the scheme gate and `check_http` again.
            .max_redirects(0)
            .timeout_global(self.timeout)
            .timeout_connect(self.connect_timeout);
        if let Some(idle) = idle {
            builder = idle::apply(builder, idle);
        }
        builder = match &self.proxy {
            ProxyChoice::Env => builder,
            ProxyChoice::Direct => builder.proxy(None),
            ProxyChoice::Url(proxy) => builder.proxy(Some(proxy.clone())),
        };
        builder.build().new_agent()
    }
}

/// `proxy` option: a `http`, `https`, `socks4`, `socks4a`, `socks5` or
/// `socks5h` URL (credentials allowed), `false`, or absent. With a security
/// policy the proxy endpoint must itself pass its `http` rules.
pub(super) fn parse_proxy(value: Option<&Value>, op: &str) -> Result<ProxyChoice, String> {
    let raw = match value {
        None | Some(Value::Null) => return Ok(ProxyChoice::Env),
        Some(Value::Bool(false)) => return Ok(ProxyChoice::Direct),
        Some(Value::String(raw)) => raw,
        Some(_) => return Err(op_error(op, "proxy must be a proxy URL string or false")),
    };
    let url = Url::parse(raw).map_err(|e| op_error(op, format!("invalid proxy URL: {e}")))?;
    if !matches!(
        url.scheme(),
        "http" | "https" | "socks4" | "socks4a" | "socks5" | "socks5h"
    ) {
        return Err(op_error(
            op,
            format!(
                "proxy scheme '{}' is not supported (use http, https, socks4, socks4a, socks5 or socks5h)",
                url.scheme()
            ),
        ));
    }
    let Some(host) = url.host_str() else {
        return Err(op_error(op, "proxy URL has no host"));
    };
    let endpoint = match url.port() {
        Some(port) => format!("{}://{host}:{port}/", url.scheme()),
        None => format!("{}://{host}/", url.scheme()),
    };
    crate::security::policy::check_http(&endpoint)?;
    ureq::Proxy::new(raw)
        .map(ProxyChoice::Url)
        .map_err(|e| op_error(op, format!("invalid proxy URL: {e}")))
}

/// `cookies` option: jar name, `"default"` for `true`, `None` when off.
pub(super) fn parse_jar(value: Option<&Value>, op: &str) -> Result<Option<String>, String> {
    match value {
        None | Some(Value::Null | Value::Bool(false)) => Ok(None),
        Some(Value::Bool(true)) => Ok(Some("default".to_owned())),
        Some(Value::String(name)) => {
            validate_jar_name(name, op)?;
            Ok(Some(name.clone()))
        },
        Some(_) => Err(op_error(op, "cookies must be true, false or a jar name")),
    }
}

pub(super) fn validate_jar_name(name: &str, op: &str) -> Result<(), String> {
    let valid = !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'));
    if valid {
        Ok(())
    } else {
        Err(op_error(
            op,
            format!(
                "invalid cookie jar name '{name}' (1-64 chars: letters, digits, '_', '-', '.')"
            ),
        ))
    }
}

/// A cookie jar: its name within the jars of one caller identity.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) struct JarKey {
    /// [`caller_scope`] of the caller that uses the jar.
    scope: String,
    name: String,
}

impl JarKey {
    /// Jar `name` of the caller dispatching on this thread.
    pub(super) fn for_caller(name: String) -> Self {
        Self {
            scope: caller_scope(),
            name,
        }
    }
}

/// Identity owning cookie jars: the backend, or one webview label showing one
/// origin (`app` for app-local content). A page never reads, plants or clears
/// the cookies of the backend, of another webview or of another origin.
fn caller_scope() -> String {
    match crate::ipc::owner::current() {
        None => "backend".to_owned(),
        Some(owner) => format!(
            "webview:{}|{}",
            owner.label(),
            owner.origin().unwrap_or("app")
        ),
    }
}

/// In-memory cookie jars by caller identity and name, for the lifetime of
/// the process. At most [`MAX_JARS`] per identity.
static JARS: LazyLock<Mutex<ahash::HashMap<JarKey, CookieStore>>> =
    LazyLock::new(|| Mutex::new(ahash::HashMap::default()));

/// `Cookie` header value of `jar` for `url`, if it holds any match.
fn jar_cookie_header(jar: &JarKey, url: &Url) -> Option<String> {
    let jars = JARS.lock();
    let store = jars.get(jar)?;
    let header = store
        .get_request_values(url)
        .map(|(name, value)| format!("{name}={value}"))
        .collect::<Vec<_>>()
        .join("; ");
    (!header.is_empty()).then_some(header)
}

/// Store the response's `Set-Cookie` headers in `jar`.
fn store_cookies(jar: &JarKey, url: &Url, response: &Response, op: &str) -> Result<(), String> {
    let mut values = response
        .headers()
        .get_all("set-cookie")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .peekable();
    if values.peek().is_none() {
        return Ok(());
    }
    let mut jars = JARS.lock();
    if !jars.contains_key(jar)
        && jars.keys().filter(|key| key.scope == jar.scope).count() >= MAX_JARS
    {
        return Err(op_error(
            op,
            format!("at most {MAX_JARS} cookie jars may exist; clear one with http.clearCookies"),
        ));
    }
    let store = jars.entry(jar.clone()).or_default();
    for value in values {
        // A malformed or foreign-domain cookie is dropped, like a browser does.
        let _ = store.parse(value, url);
    }
    Ok(())
}

/// Drop every cookie of the caller's jar `jar`, or of all the caller's jars
/// when `None`. Jars of other callers are never touched.
pub(super) fn clear_cookies(jar: Option<&str>) {
    let scope = caller_scope();
    let mut jars = JARS.lock();
    match jar {
        Some(name) => {
            jars.remove(&JarKey {
                scope,
                name: name.to_owned(),
            });
        },
        None => jars.retain(|key, _| key.scope != scope),
    }
}

struct AbortEntry {
    flag: Arc<AtomicBool>,
    owner: Option<Owner>,
    /// `false` while the abort arrived before its request registered.
    active: bool,
}

static ABORTS: LazyLock<Mutex<ahash::HashMap<String, AbortEntry>>> =
    LazyLock::new(|| Mutex::new(ahash::HashMap::default()));

/// Abort flag of one call, unregistered on drop.
pub(super) struct AbortHandle {
    id: Option<String>,
    flag: Arc<AtomicBool>,
}

fn validate_abort_id(id: &str, op: &str) -> Result<(), String> {
    let valid = !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'));
    if valid {
        Ok(())
    } else {
        Err(op_error(op, format!("invalid abortId '{id}'")))
    }
}

impl AbortHandle {
    /// Register `id` (if any) so [`abort`] can reach this call.
    pub(super) fn register(id: Option<String>, op: &str) -> Result<Self, String> {
        let Some(id) = id else {
            return Ok(Self {
                id: None,
                flag: Arc::new(AtomicBool::new(false)),
            });
        };
        validate_abort_id(&id, op)?;
        let mut aborts = ABORTS.lock();
        let flag = match aborts.get_mut(&id) {
            Some(entry) if entry.active => {
                return Err(op_error(op, format!("abortId '{id}' is already in use")));
            },
            // Aborted before it started: start aborted.
            Some(entry) if crate::ipc::owner::permits(entry.owner.as_ref()) => {
                entry.active = true;
                Arc::clone(&entry.flag)
            },
            _ => {
                let flag = Arc::new(AtomicBool::new(false));
                aborts.insert(
                    id.clone(),
                    AbortEntry {
                        flag: Arc::clone(&flag),
                        owner: crate::ipc::owner::current(),
                        active: true,
                    },
                );
                flag
            },
        };
        Ok(Self { id: Some(id), flag })
    }

    pub(super) fn is_aborted(&self) -> bool {
        self.flag.load(Ordering::Acquire)
    }

    pub(super) fn flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.flag)
    }

    /// Handle sharing this call's flag without owning its registration (its
    /// drop leaves the registry untouched).
    fn shared(&self) -> Self {
        Self {
            id: None,
            flag: self.flag(),
        }
    }

    /// `Err("aborted")` once the call was aborted.
    pub(super) fn check(&self, op: &str) -> Result<(), String> {
        if self.is_aborted() {
            Err(op_error(op, "aborted"))
        } else {
            Ok(())
        }
    }
}

impl Drop for AbortHandle {
    fn drop(&mut self) {
        if let Some(id) = self.id.take() {
            let mut aborts = ABORTS.lock();
            if aborts
                .get(&id)
                .is_some_and(|entry| Arc::ptr_eq(&entry.flag, &self.flag))
            {
                aborts.remove(&id);
            }
        }
    }
}

/// `http.abort`: flag the call registered under `id`. An id not registered
/// yet is remembered so a request whose abort overtook it starts aborted.
/// Returns whether a running call was found.
pub(super) fn abort(id: &str) -> Result<bool, String> {
    validate_abort_id(id, "abort")?;
    let mut aborts = ABORTS.lock();
    if let Some(entry) = aborts.get(id) {
        if !crate::ipc::owner::permits(entry.owner.as_ref()) {
            return Ok(false);
        }
        entry.flag.store(true, Ordering::Release);
        return Ok(entry.active);
    }
    if aborts.len() >= MAX_PENDING_ABORTS {
        aborts.retain(|_, entry| entry.active);
    }
    aborts.insert(
        id.to_owned(),
        AbortEntry {
            flag: Arc::new(AtomicBool::new(true)),
            owner: crate::ipc::owner::current(),
            active: false,
        },
    );
    Ok(false)
}

/// Reader failing with "aborted" once `flag` is set.
pub(super) struct AbortRead<R> {
    pub(super) inner: R,
    pub(super) flag: Arc<AtomicBool>,
}

impl<R: Read> Read for AbortRead<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.flag.load(Ordering::Acquire) {
            return Err(std::io::Error::other("aborted"));
        }
        self.inner.read(buf)
    }
}

/// Request body reader with upload progress and abort.
struct BodyReader {
    inner: std::io::Cursor<Bytes>,
    emitter: ProgressEmitter,
    flag: Arc<AtomicBool>,
}

impl Read for BodyReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.flag.load(Ordering::Acquire) {
            return Err(std::io::Error::other("aborted"));
        }
        let n = self.inner.read(buf)?;
        self.emitter.advance(n, n == 0);
        Ok(n)
    }
}

/// One outgoing request (method, URL, caller headers, encoded body).
pub(super) struct Outgoing {
    pub(super) method: String,
    pub(super) url: Url,
    pub(super) headers: Vec<(String, String)>,
    pub(super) body: Option<Bytes>,
    pub(super) progress_id: Option<String>,
}

/// Final response of [`execute`] with the URL it came from.
pub(super) struct Exchange {
    pub(super) response: Response,
    pub(super) url: Url,
    pub(super) redirected: bool,
}

fn is_redirect(status: u16) -> bool {
    matches!(status, 301 | 302 | 303 | 307 | 308)
}

/// Run `out`, following redirects per `net`. Every hop's URL goes through the
/// scheme gate and `check_http`; credentials are dropped when a hop changes
/// origin; cookies of `net.jar` are sent and stored on every hop.
fn execute(
    agent: &ureq::Agent,
    mut out: Outgoing,
    net: &Net,
    abort: &AbortHandle,
    op: &str,
) -> Result<Exchange, String> {
    let mut hops = 0_u32;
    loop {
        abort.check(op)?;
        let response = send_once(agent, &out, net.jar.as_ref(), abort, op)?;
        if let Some(jar) = net.jar.as_ref() {
            store_cookies(jar, &out.url, &response, op)?;
        }
        let status = response.status().as_u16();
        let location = is_redirect(status)
            .then(|| super::header_str(&response, "location"))
            .flatten()
            .map(str::to_owned);
        let Some(location) = location.filter(|_| net.redirect != RedirectMode::Manual) else {
            return Ok(Exchange {
                response,
                url: out.url,
                redirected: hops > 0,
            });
        };
        if net.redirect == RedirectMode::Error {
            return Err(op_error(
                op,
                format!(
                    "({}): redirected with status {status} to '{location}' (redirect: \"error\")",
                    out.url
                ),
            ));
        }
        if hops >= net.max_redirects {
            return Err(op_error(
                op,
                format!(
                    "({}): too many redirects (maxRedirects: {})",
                    out.url, net.max_redirects
                ),
            ));
        }
        let next = out
            .url
            .join(&location)
            .map_err(|e| op_error(op, format!("invalid redirect location '{location}': {e}")))?;
        check_redirect_target(&next, op)?;
        hops += 1;
        out = next_hop(out, next, status);
    }
}

/// [`execute`] on a helper thread while the call thread re-checks every
/// [`HEAD_POLL`] the abort flag and, with `owner`, whether the calling
/// document is still current: an abort (or a vanished document) ends the
/// call even while connecting or waiting for the response head. A helper
/// left behind ends with its transport deadlines; its result is dropped.
pub(super) fn execute_watched(
    agent: ureq::Agent,
    out: Outgoing,
    net: Net,
    abort: &AbortHandle,
    owner: Option<&Owner>,
    op: &'static str,
) -> Result<Exchange, String> {
    abort.check(op)?;
    let (tx, rx) = mpsc::sync_channel(1);
    let helper_abort = abort.shared();
    std::thread::Builder::new()
        .name("tynd-http-request".into())
        .spawn(move || {
            // The receiver is gone once the call gave up: nothing to report.
            let _ = tx.send(execute(&agent, out, &net, &helper_abort, op));
        })
        .map_err(|e| op_error(op, format!("cannot start the request thread: {e}")))?;
    loop {
        match rx.recv_timeout(HEAD_POLL) {
            Ok(result) => return result,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                abort.check(op)?;
                if owner.is_some_and(|owner| !owner.is_current()) {
                    return Err(op_error(op, "the calling document is gone"));
                }
            },
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err(op_error(op, "request thread stopped"));
            },
        }
    }
}

/// Redirect targets are limited to `http(s)` and the `security` http rules.
pub(super) fn check_redirect_target(next: &Url, op: &str) -> Result<(), String> {
    if !matches!(next.scheme(), "http" | "https") {
        return Err(op_error(
            op,
            format!(
                "redirect to scheme '{}' is not allowed (only http/https) - '{next}'",
                next.scheme()
            ),
        ));
    }
    crate::security::policy::check_http(next.as_str())
}

/// Request for the hop to `next` after a `status` redirect: 303 (and 301 /
/// 302 after a POST) turns into a body-less GET, 307 / 308 replay the request;
/// `authorization`, `cookie` and `proxy-authorization` are dropped when the
/// origin changes.
pub(super) fn next_hop(mut out: Outgoing, next: Url, status: u16) -> Outgoing {
    let to_get = (status == 303 && out.method != "HEAD")
        || (matches!(status, 301 | 302) && out.method == "POST");
    if to_get {
        "GET".clone_into(&mut out.method);
        out.body = None;
        out.headers.retain(|(name, _)| {
            !name.eq_ignore_ascii_case("content-type")
                && !name.eq_ignore_ascii_case("content-length")
        });
    }
    if next.origin() != out.url.origin() {
        out.headers.retain(|(name, _)| {
            !["authorization", "cookie", "proxy-authorization"]
                .iter()
                .any(|sensitive| name.eq_ignore_ascii_case(sensitive))
        });
    }
    out.url = next;
    out
}

fn send_once(
    agent: &ureq::Agent,
    out: &Outgoing,
    jar: Option<&JarKey>,
    abort: &AbortHandle,
    op: &str,
) -> Result<Response, String> {
    let build_error = |e: ureq::http::Error| op_error(op, format!("build request: {e}"));
    let mut builder = ureq::http::Request::builder()
        .method(out.method.as_str())
        .uri(out.url.as_str());
    let mut caller_cookie = None;
    for (name, value) in &out.headers {
        if jar.is_some() && name.eq_ignore_ascii_case("cookie") {
            caller_cookie = Some(value.as_str());
            continue;
        }
        builder = builder.header(name.as_str(), value.as_str());
    }
    let jar_cookie = jar.and_then(|jar| jar_cookie_header(jar, &out.url));
    let cookie = match (caller_cookie, jar_cookie) {
        (Some(caller), Some(jar)) => Some(format!("{caller}; {jar}")),
        (Some(caller), None) => Some(caller.to_owned()),
        (None, jar) => jar,
    };
    if let Some(cookie) = cookie {
        builder = builder.header("cookie", cookie);
    }
    let result = match &out.body {
        None => agent.run(builder.body(ureq::SendBody::none()).map_err(build_error)?),
        Some(body) => {
            let has_length = out
                .headers
                .iter()
                .any(|(name, _)| name.eq_ignore_ascii_case("content-length"));
            if !has_length {
                builder = builder.header("content-length", body.len().to_string());
            }
            let mut reader = BodyReader {
                inner: std::io::Cursor::new(body.clone()),
                emitter: ProgressEmitter::new(
                    out.progress_id.clone(),
                    "upload",
                    Some(body.len() as u64),
                ),
                flag: abort.flag(),
            };
            let request = builder
                .body(ureq::SendBody::from_reader(&mut reader))
                .map_err(build_error)?;
            agent.run(request)
        },
    };
    result.map_err(|e| {
        if abort.is_aborted() {
            op_error(op, "aborted")
        } else {
            op_error(op, format!("({}): {e}", out.url))
        }
    })
}
