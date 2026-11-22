//! Applies what native plugins registered during `init` (init scripts and
//! custom URI schemes, see [`crate::plugins::setup`]) to a webview builder.

use std::borrow::Cow;

use tynd_plugin_sdk::{UriHandler, UriRequest, UriResponse};
use wry::{
    http::{HeaderName, HeaderValue, Request, Response, StatusCode},
    WebViewBuilder,
};

use super::{
    handlers::{is_app_local_url, is_app_local_webview},
    navigation::ipc_origin,
};
use crate::plugins::setup;

/// Whether a webview created with `initial_url` starts on app-local content.
/// `None` means bundled or dev-server content.
fn initial_content_is_local(initial_url: Option<&str>) -> bool {
    initial_url.is_none_or(is_app_local_url)
}

/// `Origin` header of a request, else the origin of its `Referer`, in the
/// `scheme://host[:port]` form `ipc_origin` expects.
fn request_page_origin(request: &Request<Vec<u8>>) -> Option<String> {
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

fn uri_scheme_request(
    request: &Request<Vec<u8>>,
    webview: &str,
    origin: Option<String>,
) -> UriRequest {
    UriRequest {
        url: request.uri().to_string(),
        method: request.method().to_string(),
        headers: request
            .headers()
            .iter()
            .filter_map(|(name, value)| {
                value
                    .to_str()
                    .ok()
                    .map(|value| (name.to_string(), value.to_owned()))
            })
            .collect(),
        body: request.body().clone(),
        webview: webview.to_owned(),
        origin,
    }
}

fn uri_scheme_response(response: UriResponse) -> Response<Cow<'static, [u8]>> {
    let status = StatusCode::from_u16(response.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let mut builder = Response::builder().status(status);
    for (name, value) in &response.headers {
        if let (Ok(name), Ok(value)) = (
            HeaderName::from_bytes(name.as_bytes()),
            HeaderValue::from_str(value),
        ) {
            builder = builder.header(name, value);
        }
    }
    builder
        .body(Cow::Owned(response.body))
        .unwrap_or_else(|_| uri_scheme_failure())
}

fn uri_scheme_failure() -> Response<Cow<'static, [u8]>> {
    let mut response = Response::new(Cow::Borrowed(&b"plugin scheme handler failed"[..]));
    *response.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
    response
}

fn uri_scheme_forbidden() -> Response<Cow<'static, [u8]>> {
    let mut response = Response::new(Cow::Borrowed(&b"forbidden"[..]));
    *response.status_mut() = StatusCode::FORBIDDEN;
    response
}

/// Whether a request comes from app-local content. Navigation tracking only
/// follows the main frame, so a third-party iframe inside an app page leaves
/// the webview "local"; its `no-referrer` subresource requests carry neither
/// `Origin` nor `Referer`. Such a request is only trusted when the browser says
/// it is same-origin (or user-initiated: `none`) via `Sec-Fetch-Site`.
fn request_from_local_content(page_origin: Option<&str>, fetch_site: Option<&str>) -> bool {
    match page_origin {
        Some(origin) => is_app_local_url(origin),
        None => fetch_site.is_some_and(|site| {
            site.eq_ignore_ascii_case("same-origin") || site.eq_ignore_ascii_case("none")
        }),
    }
}

/// A plugin scheme may serve a request when the plugin opted in to remote
/// pages, or when both the webview and the requesting page are app-local.
fn scheme_request_permitted(
    remote_ok: bool,
    webview_local: bool,
    page_origin: Option<&str>,
    fetch_site: Option<&str>,
) -> bool {
    remote_ok || (webview_local && request_from_local_content(page_origin, fetch_site))
}

/// Serve one request through a plugin handler. Requests from remote pages are
/// refused unless the plugin opted in, and a panicking handler becomes a 500
/// instead of unwinding into the webview's event loop.
fn serve_uri_scheme(
    handler: &UriHandler,
    request: &Request<Vec<u8>>,
    webview: &str,
    remote_ok: bool,
) -> Response<Cow<'static, [u8]>> {
    let page_origin = request_page_origin(request);
    let fetch_site = request
        .headers()
        .get("sec-fetch-site")
        .and_then(|value| value.to_str().ok());
    if !scheme_request_permitted(
        remote_ok,
        is_app_local_webview(webview),
        page_origin.as_deref(),
        fetch_site,
    ) {
        crate::tynd_warn!(
            "plugin URI scheme request from a remote page refused (webview {webview})"
        );
        return uri_scheme_forbidden();
    }
    let origin = ipc_origin(page_origin.as_deref(), webview);
    let converted = uri_scheme_request(request, webview, origin);
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| handler(converted)))
        .map_or_else(|_| uri_scheme_failure(), uri_scheme_response)
}

/// Run `script` only in documents whose own location is app-local, so a
/// webview that later navigates to (or frames) remote content never executes
/// it. The block keeps the script's own statements unchanged.
fn guard_local_only(script: &str, dev_origin: Option<&str>) -> String {
    // Loopback is local only as the exact dev server origin.
    let dev_check = dev_origin.map_or_else(String::new, |origin| {
        let literal = serde_json::to_string(origin).unwrap_or_else(|_| "\"\"".to_owned());
        format!("||l.origin==={literal}")
    });
    format!(
        "if ((function(){{var l=window.location,h=l.hostname;\
         return l.protocol==='tynd:'||(l.protocol==='http:'&&h==='tynd.localhost'&&l.port===''){dev_check}}})()) {{\n{script}\n}}"
    )
}

/// Apply what native plugins registered during `init`: their init scripts
/// (after the Tynd shim, in registration order) and custom URI schemes.
/// Entries that did not opt in to remote content are installed only when the
/// webview opens on app-local content (see [`initial_content_is_local`]).
pub(crate) fn apply_plugin_hooks<'a>(
    mut builder: WebViewBuilder<'a>,
    label: &str,
    initial_url: Option<&str>,
) -> WebViewBuilder<'a> {
    let content_local = initial_content_is_local(initial_url);
    for entry in setup::init_script_entries() {
        if entry.remote {
            builder = builder.with_initialization_script(entry.script);
        } else if content_local {
            builder = builder.with_initialization_script(guard_local_only(
                &entry.script,
                crate::webview::navigation::dev_origin().as_deref(),
            ));
        }
    }
    for entry in setup::uri_scheme_entries() {
        if !entry.remote && !content_local {
            continue;
        }
        crate::tynd_debug!(
            "webview {label}: registering plugin URI scheme '{}'",
            entry.name
        );
        let setup::UriSchemeEntry {
            name,
            handler,
            remote,
        } = entry;
        let webview = label.to_owned();
        builder = builder.with_custom_protocol(name, move |_id, request: Request<Vec<u8>>| {
            serve_uri_scheme(&handler, &request, &webview, remote)
        });
    }
    builder
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_content_is_local_unless_it_opens_on_a_remote_url() {
        assert!(initial_content_is_local(None));
        assert!(!initial_content_is_local(Some("https://example.com")));
    }

    #[test]
    fn uri_scheme_request_and_response_convert() {
        let request = Request::builder()
            .method("POST")
            .uri("myplug://host/path?q=1")
            .header("x-test", "1")
            .body(b"body".to_vec())
            .unwrap();
        let converted = uri_scheme_request(&request, "panel", Some("https://page.test".into()));
        assert_eq!(converted.webview, "panel");
        assert_eq!(converted.origin.as_deref(), Some("https://page.test"));
        assert_eq!(converted.url, "myplug://host/path?q=1");
        assert_eq!(converted.method, "POST");
        assert_eq!(
            converted.headers,
            vec![("x-test".to_owned(), "1".to_owned())]
        );
        assert_eq!(converted.body, b"body");

        let response = uri_scheme_response(UriResponse {
            status: 201,
            headers: vec![
                ("content-type".to_owned(), "text/plain".to_owned()),
                ("bad header".to_owned(), "x".to_owned()),
            ],
            body: b"ok".to_vec(),
        });
        assert_eq!(response.status(), StatusCode::CREATED);
        assert_eq!(response.headers().len(), 1);
        assert_eq!(response.body().as_ref(), b"ok");

        let invalid = uri_scheme_response(UriResponse {
            status: 99,
            ..Default::default()
        });
        assert_eq!(invalid.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[test]
    fn panicking_scheme_handler_becomes_a_500() {
        let handler: UriHandler = std::sync::Arc::new(|_| panic!("boom"));
        let request = Request::builder()
            .uri("myplug://host/")
            .header("sec-fetch-site", "same-origin")
            .body(Vec::new())
            .unwrap();
        assert_eq!(
            serve_uri_scheme(&handler, &request, "main", false).status(),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }

    fn request_with_headers(headers: &[(&str, &str)]) -> Request<Vec<u8>> {
        let mut builder = Request::builder().uri("myplug://host/");
        for (name, value) in headers {
            builder = builder.header(*name, *value);
        }
        builder.body(Vec::new()).unwrap()
    }

    #[test]
    fn scheme_requests_from_remote_pages_are_refused_unless_opted_in() {
        let handler: UriHandler = std::sync::Arc::new(|request| UriResponse {
            status: 200,
            body: request.origin.unwrap_or_default().into_bytes(),
            ..Default::default()
        });
        let remote = request_with_headers(&[("origin", "https://evil.test")]);
        assert_eq!(
            serve_uri_scheme(&handler, &remote, "panel-a", false).status(),
            StatusCode::FORBIDDEN
        );
        let reply = serve_uri_scheme(&handler, &remote, "panel-a", true);
        assert_eq!(reply.status(), StatusCode::OK);
        assert_eq!(reply.body().as_ref(), b"https://evil.test");

        let referer = request_with_headers(&[("referer", "https://evil.test/page?x=1")]);
        assert_eq!(
            serve_uri_scheme(&handler, &referer, "panel-a", false).status(),
            StatusCode::FORBIDDEN
        );
    }

    #[test]
    fn scheme_requests_from_app_pages_are_served_with_context() {
        let handler: UriHandler = std::sync::Arc::new(|request| UriResponse {
            status: 200,
            body: format!("{}|{:?}", request.webview, request.origin).into_bytes(),
            ..Default::default()
        });
        let bundled = request_with_headers(&[("origin", "tynd://localhost")]);
        let reply = serve_uri_scheme(&handler, &bundled, "main", false);
        assert_eq!(reply.status(), StatusCode::OK);
        assert_eq!(reply.body().as_ref(), b"main|None");
        let same_origin = request_with_headers(&[("sec-fetch-site", "same-origin")]);
        assert_eq!(
            serve_uri_scheme(&handler, &same_origin, "main", false).status(),
            StatusCode::OK
        );
        // No Origin, no Referer, no same-origin proof: could be a framed remote page.
        let bare = request_with_headers(&[]);
        assert_eq!(
            serve_uri_scheme(&handler, &bare, "main", false).status(),
            StatusCode::FORBIDDEN
        );
        let cross_site = request_with_headers(&[("sec-fetch-site", "cross-site")]);
        assert_eq!(
            serve_uri_scheme(&handler, &cross_site, "main", false).status(),
            StatusCode::FORBIDDEN
        );
    }

    #[test]
    fn scheme_permission_matrix() {
        assert!(scheme_request_permitted(
            true,
            false,
            Some("https://x.test"),
            None
        ));
        assert!(scheme_request_permitted(true, true, None, None));
        assert!(!scheme_request_permitted(false, true, None, None));
        assert!(scheme_request_permitted(
            false,
            true,
            None,
            Some("same-origin")
        ));
        // A loopback origin is remote unless it is the configured dev server.
        assert!(!scheme_request_permitted(
            false,
            true,
            Some("http://localhost:5173"),
            None
        ));
        assert!(scheme_request_permitted(
            false,
            true,
            Some("tynd://localhost"),
            None
        ));
        assert!(!scheme_request_permitted(
            false,
            true,
            Some("https://x.test"),
            None
        ));
        assert!(!scheme_request_permitted(false, false, None, None));
        assert!(!scheme_request_permitted(
            false,
            false,
            Some("tynd://localhost"),
            None
        ));
    }

    #[test]
    fn page_origin_comes_from_origin_then_referer() {
        let origin = request_with_headers(&[
            ("origin", "https://a.test"),
            ("referer", "https://b.test/x"),
        ]);
        assert_eq!(
            request_page_origin(&origin).as_deref(),
            Some("https://a.test")
        );
        let referer = request_with_headers(&[("referer", "https://b.test:8443/x?y")]);
        assert_eq!(
            request_page_origin(&referer).as_deref(),
            Some("https://b.test:8443")
        );
        let opaque = request_with_headers(&[("origin", "null")]);
        assert_eq!(request_page_origin(&opaque), None);
        assert_eq!(request_page_origin(&request_with_headers(&[])), None);
    }

    #[test]
    fn guarded_init_script_wraps_the_original_statements() {
        let guarded = guard_local_only("window.__plugin = 1;", None);
        assert!(guarded.starts_with("if ((function(){"));
        assert!(guarded.contains("l.protocol==='tynd:'"));
        assert!(guarded.contains("l.protocol==='http:'&&h==='tynd.localhost'&&l.port===''"));
        assert!(!guarded.contains("127.0.0.1") && !guarded.contains("l.origin"));
        let dev = guard_local_only("x;", Some("http://localhost:5173"));
        assert!(dev.contains("||l.origin===\"http://localhost:5173\""));
        assert!(guarded.contains("{\nwindow.__plugin = 1;\n}"));
    }

    #[test]
    fn requests_without_origin_or_referer_need_a_same_origin_fetch_site() {
        assert!(!request_from_local_content(None, None));
        assert!(!request_from_local_content(None, Some("cross-site")));
        assert!(!request_from_local_content(None, Some("same-site")));
        assert!(request_from_local_content(None, Some("same-origin")));
        assert!(request_from_local_content(None, Some("none")));
        assert!(request_from_local_content(
            Some("tynd://localhost"),
            Some("cross-site")
        ));
        assert!(!request_from_local_content(
            Some("https://x.test"),
            Some("same-origin")
        ));
    }
}
