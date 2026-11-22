use super::*;

const DEV: Option<&str> = Some("http://localhost:5173");

fn frame(url: &str) -> FrameOrigin {
    classify_frame(url, Some("tynd://localhost/"), true, DEV)
}

#[test]
fn webview2_rewrite_is_the_exact_default_port_http_origin() {
    let parse = |url: &str| url::Url::parse(url).unwrap();
    for exact in [
        "http://tynd.localhost/",
        "http://tynd.localhost",
        "http://TYND.localhost/index.html?a=1",
        "http://tynd.localhost:80/",
    ] {
        assert!(is_webview2_app_origin(&parse(exact)), "{exact}");
    }
    for other in [
        "http://tynd.localhost:8080/",
        "http://tynd.localhost:443/",
        "https://tynd.localhost/",
        "https://tynd.localhost:443/",
        "http://tynd.localhost.evil.test/",
        "http://evil.tynd.localhost/",
        "http://tynd-bin.localhost/",
        "http://tynd-bin.localhost:8080/",
        "https://tynd-bin.localhost/",
        "tynd://localhost/",
        "http://u@tynd.localhost/",
        "http://u:p@tynd.localhost/",
        "http://:p@tynd.localhost/",
    ] {
        assert!(!is_webview2_app_origin(&parse(other)), "{other}");
    }
}

#[test]
fn navigation_refuses_userinfo_on_http_targets() {
    for url in [
        "http://u@tynd.localhost/",
        "http://u:p@tynd.localhost/",
        "https://u@example.com/",
        "http://u:p@localhost:5173/",
    ] {
        assert!(check_navigation_url(url).is_err(), "{url}");
    }
}

#[test]
fn non_intercepted_tynd_localhost_urls_are_remote_frames_and_documents() {
    for url in [
        "http://tynd.localhost:8080/",
        "https://tynd.localhost/",
        "http://tynd-bin.localhost/",
    ] {
        assert!(
            matches!(classify_document(url, DEV), DocumentTrust::Remote(_)),
            "{url}"
        );
        assert!(
            matches!(
                classify_frame(url, Some("tynd://localhost/"), true, DEV),
                FrameOrigin::Remote(_)
            ),
            "{url}"
        );
    }
}

#[test]
fn only_remote_frames_are_demoted() {
    for local in [
        "tynd://localhost/index.html",
        "http://localhost:5173/",
        "not a url",
    ] {
        assert_eq!(frame(local), FrameOrigin::Inherit, "{local}");
    }
    assert_eq!(
        frame("http://tynd.localhost/"),
        if cfg!(target_os = "windows") {
            FrameOrigin::Inherit
        } else {
            FrameOrigin::Remote("http://tynd.localhost".into())
        }
    );
    assert_eq!(
        frame("https://evil.test/path?q=1"),
        FrameOrigin::Remote("https://evil.test".into())
    );
    assert_eq!(
        frame("http://example.com:8080/a"),
        FrameOrigin::Remote("http://example.com:8080".into())
    );
    // A lookalike host is remote, not the WebView2 rewrite.
    assert_eq!(
        frame("https://tynd.localhost.evil.test/"),
        FrameOrigin::Remote("https://tynd.localhost.evil.test".into())
    );
    // Loopback that is not the dev server is remote.
    for loopback in [
        "http://127.0.0.1:3000/",
        "http://localhost:3000/",
        "http://[::1]:80/",
    ] {
        assert!(
            matches!(frame(loopback), FrameOrigin::Remote(_)),
            "{loopback}"
        );
    }
    assert!(matches!(
        classify_frame("http://localhost:5173/", None, true, None),
        FrameOrigin::Remote(_)
    ));
}

#[test]
fn opaque_frames_never_inherit_local_trust() {
    // srcdoc, data: and opaque blob: frames have no establishable creator.
    for opaque in [
        "about:srcdoc",
        "data:text/html,hi",
        "blob:null/1b2c",
        "about:blank",
    ] {
        assert_eq!(frame(opaque), FrameOrigin::Denied, "{opaque}");
    }
    // A blob: document carries the origin of its creator.
    assert_eq!(
        frame("blob:https://evil.test/1b2c"),
        FrameOrigin::Remote("https://evil.test".into())
    );
    assert_eq!(
        frame("blob:http://tynd.localhost/1b2c"),
        FrameOrigin::Inherit
    );
    // `about:blank` is only acceptable as the committed main document.
    assert_eq!(
        classify_frame("about:blank", Some("about:blank"), true, DEV),
        FrameOrigin::Inherit
    );
    assert_eq!(
        classify_frame("about:blank", None, true, DEV),
        FrameOrigin::Inherit
    );
    assert_eq!(
        classify_frame("about:blank", None, false, DEV),
        FrameOrigin::Denied
    );
}

#[test]
fn frame_url_demotes_a_local_webview_to_the_remote_origin() {
    let headers = wry::http::HeaderMap::new();
    let remote: wry::http::Uri = "https://evil.test/frame".parse().unwrap();
    let local: wry::http::Uri = "tynd://localhost/".parse().unwrap();
    let srcdoc: wry::http::Uri = "about:srcdoc".parse().unwrap();
    assert_eq!(
        ipc_request_origin(&headers, &remote, "origin-test-local").as_deref(),
        Some("https://evil.test")
    );
    assert_eq!(
        ipc_request_origin(&headers, &local, "origin-test-local"),
        None
    );
    // The denial origin never parses as a URL, so no capability matches it.
    let denied = ipc_request_origin(&headers, &srcdoc, "origin-test-local").unwrap();
    assert_eq!(denied, OPAQUE_ORIGIN);
    assert!(denied.parse::<url::Url>().is_err());
}

#[test]
fn documents_are_classified_by_origin() {
    assert_eq!(
        classify_document("tynd://localhost/", DEV),
        DocumentTrust::Local
    );
    assert_eq!(
        classify_document("http://localhost:5173/x", DEV),
        DocumentTrust::Local
    );
    assert_eq!(
        classify_document("http://localhost:9/", DEV),
        DocumentTrust::Remote("http://localhost:9".into())
    );
    assert_eq!(
        classify_document("http://localhost:5173/", None),
        DocumentTrust::Remote("http://localhost:5173".into())
    );
    assert_eq!(
        classify_document("about:blank", DEV),
        DocumentTrust::Inherit
    );
    assert_eq!(
        classify_document("blob:https://e.test/1", DEV),
        DocumentTrust::Inherit
    );
    assert_eq!(
        classify_document("https://e.test/a", DEV),
        DocumentTrust::Remote("https://e.test".into())
    );
}

#[test]
fn local_trust_is_granted_on_commit_not_on_navigation_start() {
    let label = "trust-commit-test";
    let handler = navigation_handler(label);

    // Remote page loads: revoked at start, stays remote after commit.
    // (The allow-list gate of the handler itself is covered elsewhere.)
    navigation_started(label, "https://evil.test/");
    assert_eq!(remote_origin(label).as_deref(), Some("https://evil.test"));
    document_committed(label, "https://evil.test/");
    assert_eq!(remote_origin(label).as_deref(), Some("https://evil.test"));

    // A navigation to tynd:// that never commits (cancelled, a download)
    // must not flip the remote document to local.
    assert!(handler("tynd://localhost/x.bin".to_string()));
    assert_eq!(remote_origin(label).as_deref(), Some("https://evil.test"));

    // Same for about:blank followed by window.stop().
    assert!(handler("about:blank".to_string()));
    assert_eq!(remote_origin(label).as_deref(), Some("https://evil.test"));
    document_committed(label, "about:blank");
    assert_eq!(remote_origin(label).as_deref(), Some("https://evil.test"));

    // Only a committed app document restores local trust.
    document_committed(label, "tynd://localhost/");
    assert_eq!(remote_origin(label), None);

    // A local page that starts a remote navigation is revoked immediately,
    // even when the navigation never commits.
    navigation_started(label, "https://other.test/");
    assert_eq!(remote_origin(label).as_deref(), Some("https://other.test"));
    clear_remote_origin(label);
}

#[test]
fn nested_blank_frame_never_inherits_trust_from_a_load_html_document() {
    let label = "load-html-nested-blank-test";
    let headers = wry::http::HeaderMap::new();
    let blank: wry::http::Uri = "about:blank".parse().unwrap();

    // `load_html` commits as `about:blank`: the document itself is local.
    document_committed(label, "about:blank");
    assert_eq!(ipc_request_origin(&headers, &blank, label), None);

    // A remote iframe loads into it: the policy hook sees that navigation
    // and revokes local trust, so a blank frame it creates is attributed
    // to the remote origin, never to the app.
    navigation_started(label, "https://evil.test/frame");
    assert_eq!(
        ipc_request_origin(&headers, &blank, label).as_deref(),
        Some("https://evil.test")
    );

    // After a bundled document commits, every blank frame is opaque.
    document_committed(label, "tynd://localhost/");
    assert_eq!(
        ipc_request_origin(&headers, &blank, label).as_deref(),
        Some(OPAQUE_ORIGIN)
    );
    clear_remote_origin(label);
}

#[test]
fn loopback_pages_are_remote_without_a_dev_server() {
    let label = "trust-loopback-test";
    navigation_started(label, "http://127.0.0.1:8080/");
    assert_eq!(
        remote_origin(label).as_deref(),
        Some("http://127.0.0.1:8080")
    );
    document_committed(label, "http://127.0.0.1:8080/");
    assert_eq!(
        remote_origin(label).as_deref(),
        Some("http://127.0.0.1:8080")
    );
    clear_remote_origin(label);
}

#[test]
fn external_protocol_handoffs_do_not_demote_a_local_page() {
    let label = "trust-external-test";
    navigation_started(label, "mailto:a@b.c");
    assert_eq!(remote_origin(label), None);
}

#[cfg(feature = "plugins")]
mod plugin_events {
    use std::sync::Arc;

    use parking_lot::Mutex;
    use serde_json::Value;
    use tynd_plugin_sdk::{PluginCx, PluginResult, TyndPlugin};

    use super::super::*;
    use crate::plugins::{self, HostEvent};

    struct NavProbe(Arc<Mutex<Vec<HostEvent>>>);

    impl TyndPlugin for NavProbe {
        fn name(&self) -> &'static str {
            "nav-probe"
        }

        fn dispatch(&self, _cx: &PluginCx, _method: &str, _args: Value) -> PluginResult<Value> {
            Ok(Value::Null)
        }

        fn on_event(&self, _cx: &PluginCx, event: &HostEvent) {
            self.0.lock().push(event.clone());
        }
    }

    #[test]
    fn allowed_navigation_reaches_plugins_and_denied_does_not() {
        let events = Arc::new(Mutex::new(Vec::new()));
        plugins::install_emit(Arc::new(|_, _| {}));
        assert!(plugins::register(Arc::new(NavProbe(Arc::clone(&events)))));

        register_webview_window("nav-probe-child", "nav-probe-win");
        let child = navigation_handler("nav-probe-child");
        let main = navigation_handler("nav-probe-main");

        assert!(child("tynd://localhost/child".to_string()));
        assert!(main("tynd://localhost/main".to_string()));
        assert!(!main("javascript:alert(1)".to_string()));

        let got = events.lock().clone();
        assert!(got.contains(&HostEvent::Navigation {
            window: "nav-probe-win".into(),
            webview: "nav-probe-child".into(),
            url: "tynd://localhost/child".into(),
        }));
        assert!(got.contains(&HostEvent::Navigation {
            window: "nav-probe-main".into(),
            webview: "nav-probe-main".into(),
            url: "tynd://localhost/main".into(),
        }));
        assert!(!got.iter().any(|event| matches!(
            event,
            HostEvent::Navigation { url, .. } if url.starts_with("javascript:")
        )));
    }
}
