//! App-declared custom protocols: the opt-in `asset://` file protocol
//! (`convertFileSrc`) and the schemes served by backend `app.protocol(...)`
//! handlers.
//!
//! Both are configured once at startup from the backend config, before any
//! webview exists, and installed on every webview by [`apply_app_protocols`].
//! Each request is served only to app-local content: a webview that navigated
//! to a remote origin, or a remote page framed inside an app page, is refused.
//! WebView2 (Windows) serves a scheme `x` as `http://x.localhost`; wry hands
//! the handler the `x://localhost/...` spelling on every OS.

mod asset;
mod backend;
mod range;

use std::borrow::Cow;

use crossbeam_channel::Sender;
use tynd_protocol::runtime::{BackendCall, BackendConfig};
use wry::{
    http::{header, HeaderValue, Request, Response, StatusCode},
    WebViewBuilder,
};

use super::handlers::{is_app_local_url, is_app_local_webview};

pub(crate) use backend::{fail as fail_backend_protocol, resolve as resolve_backend_protocol};

type ProtocolResponse = Response<Cow<'static, [u8]>>;

/// Scheme of the file protocol behind `convertFileSrc`.
const ASSET_SCHEME: &str = "asset";

/// Schemes owned by Tynd, the asset protocol or the web platform itself: an
/// `app.protocol(...)` handler cannot take them over.
const RESERVED_SCHEMES: &[&str] = &[
    "tynd",
    "tynd-bin",
    ASSET_SCHEME,
    "ipc",
    "http",
    "https",
    "ws",
    "wss",
    "file",
    "ftp",
    "about",
    "blob",
    "data",
    "javascript",
    "mailto",
    "tel",
    "view-source",
    "chrome",
    "edge",
    "webkit",
    "localhost",
];

/// Longest accepted scheme name. WebView2 serves it as a DNS label of
/// `<scheme>.localhost`, so the name follows label rules too.
const MAX_SCHEME_LEN: usize = 32;

/// A backend protocol name: lowercase ASCII letter first, then letters,
/// digits or `-` (it becomes the `http://<name>.localhost` host on Windows),
/// and not a reserved scheme.
pub(crate) fn validate_scheme_name(name: &str) -> Result<(), String> {
    let valid = name.len() <= MAX_SCHEME_LEN
        && name.starts_with(|c: char| c.is_ascii_lowercase())
        && !name.ends_with('-')
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if !valid {
        return Err(format!(
            "protocol name '{name}' is invalid: use 1-{MAX_SCHEME_LEN} lowercase letters, digits \
             or '-', starting with a letter (e.g. \"media\")"
        ));
    }
    if RESERVED_SCHEMES.contains(&name) {
        return Err(format!(
            "protocol name '{name}' is reserved; pick an app-specific name such as \"media\""
        ));
    }
    Ok(())
}

/// Install the startup `assets` scope and backend protocol schemes. Runs once,
/// before the first webview is built.
pub(crate) fn configure(
    config: &BackendConfig,
    call_tx: &Sender<BackendCall>,
) -> Result<(), String> {
    if let Some(assets) = &config.assets {
        asset::configure(&assets.scope)?;
    }
    backend::configure(&config.protocols, call_tx)
}

/// Every enabled scheme, as CSP sources for both spellings: `x:` (macOS,
/// Linux) and `http://x.localhost` (WebView2). Empty when none is enabled.
pub(crate) fn csp_sources() -> String {
    let mut schemes: Vec<&str> = Vec::new();
    if asset::enabled() {
        schemes.push(ASSET_SCHEME);
    }
    schemes.extend(backend::schemes().iter().map(String::as_str));
    csp_sources_for(&schemes)
}

fn csp_sources_for(schemes: &[&str]) -> String {
    schemes
        .iter()
        .map(|scheme| format!("{scheme}: http://{scheme}.localhost"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Register the enabled app protocols on a webview builder. `label` is the
/// webview label the app-local checks key off.
pub(crate) fn apply_app_protocols<'a>(
    mut builder: WebViewBuilder<'a>,
    label: &str,
) -> WebViewBuilder<'a> {
    if asset::enabled() {
        let webview = label.to_owned();
        builder = builder.with_asynchronous_custom_protocol(
            ASSET_SCHEME.into(),
            move |_id, request: Request<Vec<u8>>, responder| {
                asset::handle(&webview, request, responder);
            },
        );
    }
    for scheme in backend::schemes() {
        let webview = label.to_owned();
        let name = scheme.clone();
        builder = builder.with_asynchronous_custom_protocol(
            scheme.clone(),
            move |_id, request: Request<Vec<u8>>, responder| {
                backend::handle(&name, &webview, request, responder);
            },
        );
    }
    builder
}

// ---------------------------------------------------------------------------
// Request origin: shared with the plugin URI schemes.
// ---------------------------------------------------------------------------

/// `Origin` header of a request, else the origin of its `Referer`, in the
/// `scheme://host[:port]` form `ipc_origin` expects.
pub(crate) fn request_page_origin(request: &Request<Vec<u8>>) -> Option<String> {
    let header = |name: &str| {
        request
            .headers()
            .get(name)
            .and_then(|value| value.to_str().ok())
    };
    if let Some(origin) = header("origin").filter(|origin| *origin != "null") {
        return Some(origin.to_owned());
    }
    let referer = url::Url::parse(header("referer")?).ok()?;
    let host = referer.host_str()?;
    Some(match referer.port() {
        Some(port) => format!("{}://{host}:{port}", referer.scheme()),
        None => format!("{}://{host}", referer.scheme()),
    })
}

/// Whether a request comes from app-local content. Navigation tracking only
/// follows the main frame, so a third-party iframe inside an app page leaves
/// the webview "local"; its `no-referrer` subresource requests carry neither
/// `Origin` nor `Referer`. Such a request is only trusted when the browser says
/// it is same-origin (or user-initiated: `none`) via `Sec-Fetch-Site`.
pub(crate) fn request_from_local_content(
    page_origin: Option<&str>,
    fetch_site: Option<&str>,
) -> bool {
    match page_origin {
        Some(origin) => is_app_local_url(origin),
        None => fetch_site.is_some_and(|site| {
            site.eq_ignore_ascii_case("same-origin") || site.eq_ignore_ascii_case("none")
        }),
    }
}

/// An app protocol request from a remote page (or a webview showing one).
#[derive(Debug, PartialEq, Eq)]
struct Refused;

/// Gate of an app protocol request: refused when the webview or the requesting
/// page is not app-local. `Ok(cors)` carries the app-local `Origin` to echo in
/// `Access-Control-Allow-Origin` (absent for no-cors loads like `<img>`).
fn request_access(request: &Request<Vec<u8>>, webview: &str) -> Result<Option<String>, Refused> {
    let page_origin = request_page_origin(request);
    let fetch_site = request
        .headers()
        .get("sec-fetch-site")
        .and_then(|value| value.to_str().ok());
    if !(is_app_local_webview(webview)
        && request_from_local_content(page_origin.as_deref(), fetch_site))
    {
        crate::tynd_warn!(
            "app protocol request '{}' refused: not from app content (webview {webview})",
            request.uri()
        );
        return Err(Refused);
    }
    let cors = request
        .headers()
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
        .filter(|origin| is_app_local_url(origin))
        .map(str::to_owned);
    Ok(cors)
}

/// Plain-text error reply.
fn text_response(status: StatusCode, message: impl Into<Cow<'static, str>>) -> ProtocolResponse {
    let body = match message.into() {
        Cow::Borrowed(text) => Cow::Borrowed(text.as_bytes()),
        Cow::Owned(text) => Cow::Owned(text.into_bytes()),
    };
    let mut response = Response::new(body);
    *response.status_mut() = status;
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/plain; charset=utf-8"),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    response
}

fn forbidden() -> ProtocolResponse {
    text_response(
        StatusCode::FORBIDDEN,
        "app protocols only serve app content, not remote pages",
    )
}

/// Make `response` readable by the app-local page `cors` (the bundled app or
/// the dev server live on another origin than the protocol).
fn with_cors(
    mut response: ProtocolResponse,
    cors: Option<&str>,
    expose: &'static str,
) -> ProtocolResponse {
    let headers = response.headers_mut();
    headers.append(header::VARY, HeaderValue::from_static("Origin"));
    if let Some(origin) = cors.and_then(|origin| HeaderValue::from_str(origin).ok()) {
        headers.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, origin);
        headers.insert(
            header::ACCESS_CONTROL_EXPOSE_HEADERS,
            HeaderValue::from_static(expose),
        );
    }
    response
}

/// CORS preflight answer for an app-local origin; nothing is served.
fn preflight(
    request: &Request<Vec<u8>>,
    cors: Option<&str>,
    methods: &'static str,
) -> ProtocolResponse {
    let Some(origin) = cors.and_then(|origin| HeaderValue::from_str(origin).ok()) else {
        return forbidden();
    };
    let mut response = Response::new(Cow::Borrowed(&[] as &[u8]));
    *response.status_mut() = StatusCode::NO_CONTENT;
    let headers = response.headers_mut();
    headers.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, origin);
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_METHODS,
        HeaderValue::from_static(methods),
    );
    headers.insert(
        header::ACCESS_CONTROL_MAX_AGE,
        HeaderValue::from_static("600"),
    );
    headers.append(header::VARY, HeaderValue::from_static("Origin"));
    if let Some(requested) = request
        .headers()
        .get(header::ACCESS_CONTROL_REQUEST_HEADERS)
        .cloned()
    {
        headers.insert(header::ACCESS_CONTROL_ALLOW_HEADERS, requested);
    }
    response
}

#[cfg(test)]
mod tests;
