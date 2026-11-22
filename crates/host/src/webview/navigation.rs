use std::sync::LazyLock;

use dashmap::DashMap;
use serde_json::json;
use wry::{http::Request, WebViewBuilder};

use super::{scheme, scheme_bin};
use crate::{api, ipc::events::EventScope};

static REMOTE_ORIGINS: LazyLock<DashMap<String, String>> = LazyLock::new(DashMap::new);

pub(crate) fn remote_origin(label: &str) -> Option<String> {
    REMOTE_ORIGINS.get(label).map(|value| value.clone())
}

/// Forget per-webview state when a webview closes: trust bookkeeping plus the
/// OS watcher references it may still hold.
pub(crate) fn clear_remote_origin(label: &str) {
    REMOTE_ORIGINS.remove(label);
    COMMITTED_URLS.remove(label);
    WEBVIEW_WINDOWS.remove(label);
    api::release_webview(label);
}

/// URL of the last document each webview committed (page-load `Started`).
/// Opaque frame URLs (`about:blank`) are only trusted when they are the
/// committed main document itself.
static COMMITTED_URLS: LazyLock<DashMap<String, String>> = LazyLock::new(DashMap::new);

/// Origin of the dev server (`tynd dev`), the only loopback origin that counts
/// as app-local content. `None` in packaged apps: a loopback page there is a
/// remote page that happens to run on this machine.
static DEV_ORIGIN: LazyLock<parking_lot::RwLock<Option<String>>> =
    LazyLock::new(|| parking_lot::RwLock::new(None));

/// Record the configured dev server URL (call once at startup).
pub(crate) fn set_dev_origin(dev_url: Option<&str>) {
    let origin = dev_url
        .and_then(|raw| url::Url::parse(raw).ok())
        .filter(|parsed| matches!(parsed.scheme(), "http" | "https"))
        .map(|parsed| parsed.origin().ascii_serialization());
    if let Some(origin) = &origin {
        tracing::warn!(
            target: "tynd",
            "dev server {origin}: Tynd applies no Content-Security-Policy to dev-server pages;              the default CSP applies to the bundled build only"
        );
    }
    *DEV_ORIGIN.write() = origin;
}

pub(crate) fn dev_origin() -> Option<String> {
    DEV_ORIGIN.read().clone()
}

/// Origin reported for a caller that must not match any capability: opaque
/// frames and unparsable URLs. It is not a parsable URL, so no remote pattern
/// matches it.
pub(crate) const OPAQUE_ORIGIN: &str = "null";

/// Trust a document URL earns the webview that shows it.
#[derive(Debug, PartialEq, Eq)]
enum DocumentTrust {
    /// Bundled `tynd://` content or the configured dev server.
    Local,
    /// Any other page, identified by its serialized origin.
    Remote(String),
    /// `about:blank` / `blob:` / `data:`: the document inherits its creator's
    /// trust, so the tracked state stays as it is.
    Inherit,
}

fn classify_document(url: &str, dev_origin: Option<&str>) -> DocumentTrust {
    if url.is_empty() {
        return DocumentTrust::Inherit;
    }
    let Ok(parsed) = url::Url::parse(url) else {
        return DocumentTrust::Remote(OPAQUE_ORIGIN.to_owned());
    };
    if is_internal_app_url_parsed(&parsed) {
        return DocumentTrust::Local;
    }
    match parsed.scheme() {
        "about" | "blob" | "data" => DocumentTrust::Inherit,
        _ => {
            let origin = parsed.origin().ascii_serialization();
            if matches!(parsed.scheme(), "http" | "https") && dev_origin == Some(origin.as_str()) {
                DocumentTrust::Local
            } else {
                DocumentTrust::Remote(origin)
            }
        },
    }
}

/// A navigation is starting. It may be cancelled, turned into a download or
/// never commit, so it can only *revoke* trust (a non-local target marks the
/// webview remote); trust is granted back by [`document_committed`].
fn navigation_started(label: &str, url: &str) {
    // Only web navigations replace the document; `mailto:` and other external
    // protocol hand-offs never commit and must not demote the page.
    let is_web =
        url::Url::parse(url).is_ok_and(|parsed| matches!(parsed.scheme(), "http" | "https"));
    if !is_web {
        return;
    }
    if let DocumentTrust::Remote(origin) = classify_document(url, dev_origin().as_deref()) {
        REMOTE_ORIGINS.insert(label.to_owned(), origin);
    }
}

/// A document was committed in the main frame of `label` (page-load
/// `Started`, which wry fires on commit on every platform). This is the only
/// place that grants app-local trust.
pub(crate) fn document_committed(label: &str, url: &str) {
    // The previous document is gone (navigation or reload), and with it the
    // JS that would have balanced its watcher subscriptions.
    api::release_webview(label);
    COMMITTED_URLS.insert(label.to_owned(), url.to_owned());
    match classify_document(url, dev_origin().as_deref()) {
        DocumentTrust::Local => {
            REMOTE_ORIGINS.remove(label);
        },
        DocumentTrust::Remote(origin) => {
            REMOTE_ORIGINS.insert(label.to_owned(), origin);
        },
        DocumentTrust::Inherit => {},
    }
}

/// Owning window of child webviews, so `HostEvent::Navigation` carries the
/// right `window`. Primary and secondary windows host a webview labelled like
/// the window and need no entry.
static WEBVIEW_WINDOWS: LazyLock<DashMap<String, String>> = LazyLock::new(DashMap::new);

/// Record that child webview `webview` lives in window `window`.
pub(crate) fn register_webview_window(webview: &str, window: &str) {
    WEBVIEW_WINDOWS.insert(webview.to_string(), window.to_string());
}

#[cfg(feature = "plugins")]
fn window_of_webview(webview: &str) -> String {
    WEBVIEW_WINDOWS
        .get(webview)
        .map_or_else(|| webview.to_string(), |window| window.clone())
}

/// Tell native plugins about a navigation decision (allowed only).
fn broadcast_navigation(webview: &str, url: &str) {
    #[cfg(feature = "plugins")]
    crate::plugins::broadcast_event(&crate::plugins::HostEvent::Navigation {
        window: window_of_webview(webview),
        webview: webview.to_string(),
        url: url.to_string(),
    });
    #[cfg(not(feature = "plugins"))]
    let _ = (webview, url);
}

/// Normalize the renderer origin before capability checks. WebView2 rewrites
/// `tynd://localhost/...` to `http://tynd.localhost/...` internally so its
/// web-resource interception API can serve custom protocols. That rewritten
/// origin is still bundled/local content and must not be treated as a remote
/// website.
pub(crate) fn ipc_origin(raw: Option<&str>, label: &str) -> Option<String> {
    match raw {
        Some(origin) if is_internal_app_url(origin) => None,
        Some(origin) => Some(origin.to_string()),
        None => remote_origin(label),
    }
}

/// How the origin of the frame that posted an IPC message is judged.
#[derive(Debug, PartialEq, Eq)]
enum FrameOrigin {
    /// A remote page: the caller is that origin.
    Remote(String),
    /// App-local (or unreadable) frame: main-frame tracking decides.
    Inherit,
    /// An opaque document (`srcdoc`, `data:`, `blob:` without a tuple origin,
    /// a stray `about:blank` frame): no creator can be established, deny.
    Denied,
}

fn classify_frame(
    url: &str,
    committed: Option<&str>,
    main_is_local: bool,
    dev_origin: Option<&str>,
) -> FrameOrigin {
    let Ok(parsed) = url::Url::parse(url) else {
        return FrameOrigin::Inherit;
    };
    match parsed.scheme() {
        // `blob:https://evil/..` carries its creator's origin.
        "blob" => match parsed.origin() {
            url::Origin::Tuple(..) => classify_frame(
                &parsed.origin().ascii_serialization(),
                committed,
                main_is_local,
                dev_origin,
            ),
            url::Origin::Opaque(_) => FrameOrigin::Denied,
        },
        "about" => {
            // Only the main document itself may be `about:blank`; any other
            // blank frame was created by a script of unknown trust.
            let is_main = committed.map_or(main_is_local, |main| main == url);
            if url == "about:blank" && is_main {
                FrameOrigin::Inherit
            } else {
                FrameOrigin::Denied
            }
        },
        "data" => FrameOrigin::Denied,
        _ if is_internal_app_url_parsed(&parsed) => FrameOrigin::Inherit,
        _ => {
            let origin = parsed.origin().ascii_serialization();
            if matches!(parsed.scheme(), "http" | "https") && dev_origin == Some(origin.as_str()) {
                FrameOrigin::Inherit
            } else {
                FrameOrigin::Remote(origin)
            }
        },
    }
}

/// Caller origin of a webview IPC message.
///
/// wry reports the URL of the frame that posted the message as the request
/// URI (macOS `frameInfo`, WebView2 `Source`), so a remote iframe embedded in
/// app-local content is classified by its own origin instead of inheriting the
/// main page's trust. Opaque frames (`about:srcdoc`, `data:`, stray
/// `about:blank`) are denied and `blob:` frames take their creator's origin.
/// An app-local frame URL can never promote a caller: the main-frame
/// navigation tracking decides.
///
/// On Linux (WebKitGTK) wry cannot report the frame URL and passes the
/// main-frame URL. A cross-origin iframe still cannot post as the main frame:
/// the IPC handler drops any message without a per-webview token, and only
/// the app-token holder (an app document in the main frame) gets app trust
/// (see [`super::builder::with_tynd_runtime`]).
pub(crate) fn ipc_request_origin(
    headers: &wry::http::HeaderMap,
    uri: &wry::http::Uri,
    label: &str,
) -> Option<String> {
    let header = headers.get("origin").and_then(|value| value.to_str().ok());
    if header.is_some() {
        return ipc_origin(header, label);
    }
    let committed = COMMITTED_URLS.get(label).map(|url| url.clone());
    match classify_frame(
        &uri.to_string(),
        committed.as_deref(),
        crate::webview::handlers::is_app_local_webview(label),
        dev_origin().as_deref(),
    ) {
        FrameOrigin::Remote(origin) => Some(origin),
        FrameOrigin::Denied => Some(OPAQUE_ORIGIN.to_owned()),
        FrameOrigin::Inherit => ipc_origin(None, label),
    }
}

/// RFC 3986 scheme of a `scheme://...` string, at least two characters long so
/// a Windows drive path (`C:\a`, `C://a`) is never mistaken for a URL.
fn url_scheme(s: &str) -> Option<&str> {
    let (scheme, _) = s.split_once("://")?;
    let mut chars = scheme.chars();
    let valid = scheme.len() >= 2
        && chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
    valid.then_some(scheme)
}

/// Deep-link detector for argv and OS open-url events. Accepts any
/// `scheme://...` URL, so custom schemes (`myapp://open?id=1`) reach the app.
/// Hard-rejects `javascript`, `data`, `vbscript`, `about`, `chrome`,
/// `chrome-extension`, `blob`, `view-source` -- without this guard a hostile
/// argv `myapp javascript://...` would reach the `app:open-url` handler.
pub(crate) fn looks_like_url(s: &str) -> bool {
    if url_scheme(s).is_none() {
        return false;
    }
    let Ok(parsed) = url::Url::parse(s) else {
        return false;
    };

    // Hard deny: these schemes are dangerous as deep-link payloads even when
    // they parse, and no legitimate caller should emit them.
    if matches!(
        parsed.scheme(),
        "javascript"
            | "data"
            | "vbscript"
            | "about"
            | "chrome"
            | "chrome-extension"
            | "blob"
            | "view-source"
    ) {
        return false;
    }

    if parsed.scheme() == "file" {
        // Reject `file://./relative` / `file://../escape` and any URL whose
        // host segment contains traversal markers. `Url::parse` accepts these
        // as opaque hosts; we don't.
        let host = parsed.host_str().unwrap_or("");
        if host == "." || host == ".." || host.contains("./") || host.contains("../") {
            return false;
        }
    }
    true
}

/// Navigation hook: emits `webview:navigation` and blocks the load when the
/// security policy denies the URL. Remote HTTP(S) navigation requires an
/// explicitly configured allow-list; bundled and loopback development origins
/// remain available before policy configuration.
pub(crate) fn navigation_handler(label: &str) -> impl Fn(String) -> bool + 'static {
    let label = label.to_string();
    move |url: String| {
        // Hard-deny dangerous schemes regardless of policy state. The
        // `__tynd__` shim is injected into every main-frame document the
        // WebView loads, so a navigation to `javascript:` / `data:` /
        // `file://` etc. with an unconfigured policy would expose the IPC
        // surface to attacker-controlled JS. Keep the list aligned with
        // `is_safe_navigation_url`.
        let allowed = check_navigation_url(&url).is_ok();
        if allowed {
            navigation_started(&label, &url);
        }
        // The URL may carry OAuth codes / tokens: keep it away from remote pages.
        crate::ipc::events::emit_scoped(
            "webview:navigation",
            &json!({ "label": label, "url": url, "allowed": allowed }),
            EventScope::Origin(label.clone()),
        );
        if allowed {
            broadcast_navigation(&label, &url);
        }
        allowed
    }
}

pub(crate) fn check_navigation_url(url: &str) -> Result<(), String> {
    if !is_safe_navigation_url(url) {
        return Err(format!("unsafe navigation URL '{url}'"));
    }
    let Ok(parsed) = url::Url::parse(url) else {
        return Err(format!("invalid navigation URL '{url}'"));
    };
    if matches!(parsed.scheme(), "http" | "https")
        && (!parsed.username().is_empty() || parsed.password().is_some())
    {
        return Err(format!("navigation URL with credentials '{url}'"));
    }
    if is_internal_app_url_parsed(&parsed) {
        return Ok(());
    }
    match parsed.scheme() {
        "about" => Ok(()),
        "http" | "https" if is_loopback_host(&parsed) => crate::security::policy::check_http(url),
        "http" | "https" => crate::security::policy::check_http_configured(url),
        _ => crate::security::policy::check_http(url),
    }
}

/// Resolve a root-relative frontend route against a trusted current/base URL,
/// then apply the same policy as an absolute navigation.
pub(crate) fn resolve_navigation_url(target: &str, base: &str) -> Result<String, String> {
    if url::Url::parse(target).is_ok() {
        check_navigation_url(target)?;
        return Ok(target.to_string());
    }
    if !target.starts_with('/') || target.starts_with("//") || target.contains('\\') {
        return Err(format!("invalid navigation URL '{target}'"));
    }
    let base =
        url::Url::parse(base).map_err(|_| format!("invalid navigation base URL '{base}'"))?;
    let resolved = base
        .join(target)
        .map_err(|_| format!("invalid navigation URL '{target}'"))?
        .to_string();
    check_navigation_url(&resolved)?;
    Ok(resolved)
}

pub(crate) fn configure_webview_navigation<'a>(
    mut builder: WebViewBuilder<'a>,
    custom_url: Option<&str>,
    dev_url: Option<&str>,
    frontend_dir: Option<&str>,
    error_context: &str,
) -> Result<WebViewBuilder<'a>, String> {
    builder = builder.with_asynchronous_custom_protocol(
        "tynd-bin".into(),
        |id, request: Request<Vec<u8>>, responder| scheme_bin::handle_async(id, request, responder),
    );
    if let Some(dir) = frontend_dir {
        let dir = dir.to_string();
        builder = builder
            .with_custom_protocol("tynd".into(), move |_id, request: Request<Vec<u8>>| {
                scheme::handle(&dir, &request)
            });
    }

    if let Some(url) = custom_url {
        let base = dev_url.unwrap_or("tynd://localhost/");
        let resolved = resolve_navigation_url(url, base)
            .map_err(|error| format!("{error_context}: {error}"))?;
        Ok(builder.with_url(&resolved))
    } else if let Some(url) = dev_url {
        Ok(builder.with_url(url))
    } else if frontend_dir.is_some() {
        Ok(builder.with_url("tynd://localhost/"))
    } else {
        Ok(builder)
    }
}
/// Parse-time validation for a URL whose runtime base is not known yet.
pub(crate) fn check_navigation_target(target: &str) -> Result<(), String> {
    if url::Url::parse(target).is_ok() {
        check_navigation_url(target)
    } else if target.starts_with('/') && !target.starts_with("//") && !target.contains('\\') {
        Ok(())
    } else {
        Err(format!("invalid navigation URL '{target}'"))
    }
}

fn is_loopback_host(url: &url::Url) -> bool {
    let Some(host) = url.host_str() else {
        return false;
    };
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}

fn is_internal_app_url(url: &str) -> bool {
    url::Url::parse(url).is_ok_and(|parsed| is_internal_app_url_parsed(&parsed))
}

/// The exact origin WebView2 serves bundled content from: wry intercepts only
/// `http://tynd.localhost/*` on the default port. Any other scheme or port on
/// that host (`https://tynd.localhost`, `http://tynd.localhost:8080`) is not
/// intercepted, reaches whatever listens on loopback (`*.localhost` resolves
/// there) and is remote content. `tynd.localhost.evil.example` is remote too.
pub(crate) fn is_webview2_app_origin(url: &url::Url) -> bool {
    url.scheme() == "http"
        && url.port().is_none()
        && url.username().is_empty()
        && url.password().is_none()
        && url
            .host_str()
            .is_some_and(|host| host.eq_ignore_ascii_case("tynd.localhost"))
}

fn is_internal_app_url_parsed(url: &url::Url) -> bool {
    // The WebView2 rewrite exists on Windows only; elsewhere
    // `http://tynd.localhost` is an ordinary remote page.
    url.scheme() == "tynd" || (cfg!(target_os = "windows") && is_webview2_app_origin(url))
}

/// Reject schemes that bypass the host's origin model (`javascript:`,
/// `data:`, etc.) before consulting the deployer policy. `about:blank` is the
/// one exception: wry uses it as a transient initial document.
pub(crate) fn is_safe_navigation_url(url: &str) -> bool {
    if url == "about:blank" {
        return true;
    }
    let lower = url.to_ascii_lowercase();
    const DANGEROUS: &[&str] = &[
        "javascript:",
        "data:",
        "vbscript:",
        "blob:",
        "view-source:",
        "chrome:",
        "chrome-extension:",
        "about:",
        "file:",
    ];
    !DANGEROUS.iter().any(|p| lower.starts_with(p))
}

#[cfg(test)]
mod tests;
