use std::collections::HashSet;

use super::*;

#[test]
fn runtime_config_injects_the_registered_bin_origin() {
    let script = inject_runtime_config();
    let expected = if cfg!(target_os = "windows") {
        "http://tynd-bin.localhost"
    } else {
        "tynd-bin://localhost"
    };
    assert!(
        script.contains(&format!(
            "globalThis.__TYND_BIN_ORIGIN__ = JSON.parse('\"{expected}\"');"
        )),
        "{script}"
    );
}

#[test]
fn new_window_policy_parses_known_values_only() {
    assert_eq!(NewWindowPolicy::parse("deny"), Some(NewWindowPolicy::Deny));
    assert_eq!(
        NewWindowPolicy::parse("external"),
        Some(NewWindowPolicy::External)
    );
    assert_eq!(
        NewWindowPolicy::parse("allow"),
        Some(NewWindowPolicy::Allow)
    );
    assert_eq!(NewWindowPolicy::parse("Allow"), None);
    assert_eq!(NewWindowPolicy::parse(""), None);
    assert_eq!(NewWindowPolicy::default(), NewWindowPolicy::Allow);
}

#[test]
fn deny_policy_blocks_everything() {
    assert_eq!(
        decide_new_window(NewWindowPolicy::Deny, "https://example.com/"),
        NewWindowDecision::Deny
    );
}

#[test]
fn external_policy_opens_web_and_mail_only() {
    for url in [
        "https://example.com/a?b=1",
        "http://example.com",
        "mailto:a@b.c",
    ] {
        assert_eq!(
            decide_new_window(NewWindowPolicy::External, url),
            NewWindowDecision::OpenExternal(url.to_owned()),
            "{url}"
        );
    }
    for url in [
        "javascript:alert(1)",
        "file:///etc/passwd",
        "data:text/html,hi",
        "tynd://localhost/",
        "ms-msdt:/id",
        "about:blank",
        "not a url",
        "http://",
    ] {
        assert_eq!(
            decide_new_window(NewWindowPolicy::External, url),
            NewWindowDecision::Deny,
            "{url}"
        );
    }
}

#[test]
fn allow_policy_still_applies_the_navigation_gate() {
    assert_eq!(
        decide_new_window(NewWindowPolicy::Allow, "about:blank"),
        NewWindowDecision::AllowDefault
    );
    assert_eq!(
        decide_new_window(NewWindowPolicy::Allow, "tynd://localhost/page"),
        NewWindowDecision::AllowDefault
    );
    for url in ["javascript:alert(1)", "data:text/html,hi", "file:///c:/x"] {
        assert_eq!(
            decide_new_window(NewWindowPolicy::Allow, url),
            NewWindowDecision::Deny,
            "{url}"
        );
    }
}

#[test]
fn external_open_is_throttled() {
    let last = AtomicU64::new(0);
    assert!(external_open_permitted(1_000, &last));
    assert!(!external_open_permitted(
        1_000 + EXTERNAL_OPEN_MIN_GAP_MS - 1,
        &last
    ));
    assert!(external_open_permitted(
        1_000 + EXTERNAL_OPEN_MIN_GAP_MS,
        &last
    ));
}

#[test]
fn download_names_are_reduced_to_a_safe_component() {
    assert_eq!(sanitize_download_name("report.pdf"), "report.pdf");
    assert_eq!(sanitize_download_name("../../etc/passwd"), "passwd");
    assert_eq!(sanitize_download_name(r"..\..\evil.exe"), "evil.exe");
    assert_eq!(sanitize_download_name("a<b>:c|d?.txt"), "abcd.txt");
    assert_eq!(sanitize_download_name("  trailing. . "), "trailing");
    assert_eq!(sanitize_download_name(""), "download");
    assert_eq!(sanitize_download_name("..."), "download");
    assert_eq!(sanitize_download_name("CON.txt"), "_CON.txt");
    assert_eq!(sanitize_download_name("lpt1"), "_lpt1");
    assert_eq!(sanitize_download_name("com10.txt"), "com10.txt");
    assert_eq!(
        sanitize_download_name(&"x".repeat(500)).chars().count(),
        200
    );
    assert_eq!(sanitize_download_name("tab\there.txt"), "tabhere.txt");
}

#[test]
fn unique_destination_appends_a_counter_before_the_extension() {
    let dir = Path::new("/dl");
    let taken: HashSet<PathBuf> = [
        PathBuf::from("/dl/a.tar.gz"),
        PathBuf::from("/dl/a.tar (1).gz"),
        PathBuf::from("/dl/noext"),
    ]
    .into();
    let exists = |p: &Path| taken.contains(p);
    assert_eq!(
        unique_destination(dir, "fresh.txt", exists),
        Some(PathBuf::from("/dl/fresh.txt"))
    );
    assert_eq!(
        unique_destination(dir, "a.tar.gz", exists),
        Some(PathBuf::from("/dl/a.tar (2).gz"))
    );
    assert_eq!(
        unique_destination(dir, "noext", exists),
        Some(PathBuf::from("/dl/noext (1)"))
    );
    assert_eq!(unique_destination(dir, "x.txt", |_| true), None);
}

#[test]
fn download_destination_always_lands_in_the_downloads_dir() {
    let downloads = Some(PathBuf::from("/home/u/Downloads"));
    assert_eq!(
        download_destination(Path::new("/tmp/elsewhere/file.zip"), downloads, |_| false),
        Some(PathBuf::from("/home/u/Downloads/file.zip"))
    );
    let absolute = std::env::temp_dir().join("file.zip");
    assert_eq!(
        download_destination(&absolute, None, |_| false),
        Some(absolute.clone())
    );
    assert_eq!(
        download_destination(Path::new("file.zip"), None, |_| false),
        None
    );
}

#[test]
fn proxy_spec_accepts_http_and_socks5() {
    let http = ProxySpec::parse("http://proxy.local:3128").unwrap();
    assert_eq!(http.kind, ProxyKind::Http);
    assert_eq!(
        (http.host.as_str(), http.port.as_str()),
        ("proxy.local", "3128")
    );
    let socks = ProxySpec::parse("socks5://127.0.0.1:1080/").unwrap();
    assert_eq!(socks.kind, ProxyKind::Socks5);
    assert_eq!(socks.port, "1080");
    assert_eq!(ProxySpec::parse("http://proxy.local").unwrap().port, "80");
    assert!(matches!(http.to_wry(), ProxyConfig::Http(_)));
    assert!(matches!(socks.to_wry(), ProxyConfig::Socks5(_)));
}

#[test]
fn proxy_spec_rejects_unsupported_shapes() {
    for raw in [
        "socks5://proxy.local",
        "https://proxy.local:443",
        "socks4://proxy.local:1080",
        "http://user:pw@proxy.local:3128",
        "http://proxy.local:3128/path",
        "http://proxy.local:3128/?q=1",
        "proxy.local:3128",
        "",
    ] {
        assert!(ProxySpec::parse(raw).is_err(), "{raw}");
    }
}

#[test]
fn webview_policy_defaults_and_validation() {
    let default = WebviewPolicy::from_raw(None, None, None, "create").unwrap();
    assert_eq!(default.new_window, NewWindowPolicy::Allow);
    assert_eq!(default.downloads, None);
    assert_eq!(default.proxy, None);
    assert_eq!(
        WebviewPolicy::from_raw(Some(""), Some(""), Some(""), "create").unwrap(),
        default
    );

    let error = WebviewPolicy::from_raw(Some("popup"), None, None, "create").unwrap_err();
    assert!(error.starts_with("create: newWindow"), "{error}");
    let error = WebviewPolicy::from_raw(None, Some("ask"), None, "create").unwrap_err();
    assert!(error.starts_with("create: downloads"), "{error}");
    let error = WebviewPolicy::from_raw(None, None, Some("ftp://x:1"), "create").unwrap_err();
    assert!(error.starts_with("create: "), "{error}");
}

#[test]
fn webview_policy_reads_create_arguments() {
    let configured = WebviewPolicy::from_args(
        &json!({ "newWindow": "deny", "downloads": "allow" }),
        "create",
    )
    .unwrap();
    assert_eq!(configured.new_window, NewWindowPolicy::Deny);
    assert_eq!(configured.downloads, Some(DownloadPolicy::Allow));
    assert_eq!(configured.proxy, None);

    let proxied =
        WebviewPolicy::from_args(&json!({ "proxy": "socks5://127.0.0.1:9050" }), "create");
    assert_eq!(proxied.unwrap().proxy.unwrap().kind, ProxyKind::Socks5);
}

#[test]
fn primary_policy_reads_window_config() {
    let default: crate::window::WindowConfig = serde_json::from_str("{}").unwrap();
    assert_eq!(
        WebviewPolicy::for_primary(&default).unwrap(),
        WebviewPolicy::default()
    );

    let configured: crate::window::WindowConfig =
        serde_json::from_value(json!({ "newWindow": "external", "downloads": "deny" })).unwrap();
    let policy = WebviewPolicy::for_primary(&configured).unwrap();
    assert_eq!(policy.new_window, NewWindowPolicy::External);
    assert_eq!(policy.downloads, Some(DownloadPolicy::Deny));

    let invalid: crate::window::WindowConfig =
        serde_json::from_value(json!({ "newWindow": "popup" })).unwrap();
    let error = WebviewPolicy::for_primary(&invalid).unwrap_err();
    assert!(error.starts_with("window: newWindow"), "{error}");
}

#[test]
fn cookie_round_trips_through_json() {
    let cookie = cookie_from_json(
        &json!({
            "name": "sid", "value": "abc", "domain": "example.com", "path": "/app",
            "secure": true, "httpOnly": true, "sameSite": "lax", "expires": 4_102_444_800_i64,
        }),
        None,
        "set",
    )
    .unwrap();
    let out = cookie_to_json(&cookie);
    assert_eq!(out["name"], "sid");
    assert_eq!(out["value"], "abc");
    assert_eq!(out["domain"], "example.com");
    assert_eq!(out["path"], "/app");
    assert_eq!(out["secure"], true);
    assert_eq!(out["httpOnly"], true);
    assert_eq!(out["sameSite"], "lax");
    assert_eq!(out["expires"], 4_102_444_800_i64);
}

#[test]
fn cookie_domain_falls_back_to_the_url_host() {
    let url = parse_http_url("https://app.example.com/x", "set").unwrap();
    let cookie =
        cookie_from_json(&json!({ "name": "a", "value": "b" }), Some(&url), "set").unwrap();
    assert_eq!(cookie.domain(), Some("app.example.com"));
    assert_eq!(cookie.path(), Some("/"));
    let error = cookie_from_json(&json!({ "name": "a", "value": "b" }), None, "set").unwrap_err();
    assert!(error.contains("domain"), "{error}");
    let key = cookie_key_from_json(&json!({ "name": "a" }), Some(&url), "delete").unwrap();
    assert_eq!((key.name(), key.domain()), ("a", Some("app.example.com")));
}

#[test]
fn cookie_input_is_validated() {
    let domain = json!("example.com");
    for bad in [
        json!({ "name": "", "value": "v", "domain": domain }),
        json!({ "name": "a b", "value": "v", "domain": domain }),
        json!({ "name": "a;b", "value": "v", "domain": domain }),
        json!({ "name": "a", "value": "v;Domain=evil", "domain": domain }),
        json!({ "name": "a", "value": "v", "domain": "exa mple.com" }),
        json!({ "name": "a", "value": "v", "domain": domain, "path": "no-slash" }),
        json!({ "name": "a", "value": "v", "domain": domain, "sameSite": "weird" }),
        json!({ "name": "a", "value": "v", "domain": domain, "expires": "soon" }),
        json!({ "name": "a", "value": 1, "domain": domain }),
    ] {
        assert!(cookie_from_json(&bad, None, "set").is_err(), "{bad}");
    }
}

#[test]
fn webview2_rewrite_is_local_only_on_windows_and_only_when_exact() {
    for url in ["http://tynd.localhost/x", "http://tynd.localhost"] {
        assert_eq!(
            is_app_local_url_with(url, None),
            cfg!(target_os = "windows"),
            "{url}"
        );
    }
    assert!(!is_app_local_url_with("http://tynd.localhost:8080/", None));
    assert!(!is_app_local_url_with("https://tynd.localhost/", None));
}

#[test]
fn app_local_urls_are_bundled_or_the_dev_server() {
    let dev = Some("http://localhost:5173");
    for url in [
        "tynd://localhost/",
        "tynd://localhost/page",
        "http://localhost:5173/",
        "http://localhost:5173/a/b?c=1",
        "about:blank",
    ] {
        assert!(is_app_local_url_with(url, dev), "{url}");
    }
    for url in [
        "https://example.com",
        "https://tynd.localhost",
        "http://tynd.localhost:8080/",
        "http://tynd.localhost:3000/x",
        "https://tynd.localhost:443/",
        "http://tynd-bin.localhost/",
        "http://tynd-bin.localhost:8080/",
        "http://tynd.localhost.evil.test/",
        "http://localhost.evil.test/",
        "http://10.0.0.5:3000/",
        // Loopback is remote unless it is exactly the dev server origin.
        "http://localhost:3000/",
        "http://localhost:5173.evil.test/",
        "https://localhost:5173/",
        "http://127.0.0.1:5173/",
        "http://127.0.0.1:3000",
        "http://[::1]:8080/",
        "file:///etc/passwd",
        "javascript:alert(1)",
        "about:srcdoc",
        "not a url",
        "",
    ] {
        assert!(!is_app_local_url_with(url, dev), "{url}");
    }
    // Packaged app: no dev server, so no loopback origin is local.
    for url in [
        "http://localhost:5173/",
        "http://127.0.0.1:3000",
        "http://[::1]:8080/",
    ] {
        assert!(!is_app_local_url_with(url, None), "{url}");
    }
}

#[test]
fn download_policy_parses_and_defaults_by_content() {
    assert_eq!(DownloadPolicy::parse("allow"), Some(DownloadPolicy::Allow));
    assert_eq!(DownloadPolicy::parse("deny"), Some(DownloadPolicy::Deny));
    assert_eq!(DownloadPolicy::parse("Allow"), None);
    assert_eq!(DownloadPolicy::parse(""), None);

    let local = "tynd://localhost/report.csv";
    assert!(downloads_permitted(None, true, local));
    assert!(downloads_permitted(
        None,
        true,
        "blob:tynd://localhost/1b2c"
    ));
    assert!(!downloads_permitted(None, false, local));
    assert!(!downloads_permitted(
        None,
        true,
        "https://cdn.example/file.zip"
    ));
    assert!(!downloads_permitted(None, true, "blob:https://evil.test/x"));
    // A data: URL names no provenance: a remote iframe inside local content
    // can start one, so only an explicit `allow` policy accepts it.
    assert!(!downloads_permitted(None, true, "data:text/csv,a%2Cb"));
    assert!(!downloads_permitted(None, false, "data:text/csv,a%2Cb"));
    assert!(!downloads_permitted(
        None,
        true,
        "DATA:text/csv;base64,YQ=="
    ));
    assert!(downloads_permitted(
        Some(DownloadPolicy::Allow),
        true,
        "data:text/csv,a%2Cb"
    ));
    assert!(!downloads_permitted(
        Some(DownloadPolicy::Deny),
        true,
        "data:text/csv,a%2Cb"
    ));
    assert!(downloads_permitted(
        Some(DownloadPolicy::Allow),
        false,
        "https://cdn.example/file.zip"
    ));
    assert!(!downloads_permitted(
        Some(DownloadPolicy::Deny),
        true,
        local
    ));
}

#[test]
fn download_names_lose_bidi_and_invisible_characters() {
    assert_eq!(
        sanitize_download_name("invoice\u{202E}gpj.exe"),
        "invoicegpj.exe"
    );
    assert_eq!(
        sanitize_download_name("a\u{200B}b\u{2066}c\u{2069}d\u{FEFF}.txt"),
        "abcd.txt"
    );
    assert_eq!(sanitize_download_name("\u{202E}\u{200F}"), "download");
    assert_eq!(sanitize_download_name("caf\u{e9}.txt"), "caf\u{e9}.txt");
}

#[test]
fn concurrent_downloads_never_share_a_destination() {
    let reservations = DownloadReservations::default();
    let dir = Some(PathBuf::from("/dl"));
    let suggested = Path::new("/tmp/report.pdf");
    let first = reservations
        .reserve(suggested, dir.clone(), |_| false)
        .unwrap();
    let second = reservations
        .reserve(suggested, dir.clone(), |_| false)
        .unwrap();
    let third = reservations
        .reserve(suggested, dir.clone(), |_| false)
        .unwrap();
    assert_eq!(first, PathBuf::from("/dl/report.pdf"));
    assert_eq!(second, PathBuf::from("/dl/report (1).pdf"));
    assert_eq!(third, PathBuf::from("/dl/report (2).pdf"));

    reservations.release(&second);
    assert_eq!(
        reservations.reserve(suggested, dir, |_| false).unwrap(),
        second
    );
}

#[test]
fn reservations_also_respect_files_already_on_disk() {
    let reservations = DownloadReservations::default();
    let dir = Some(PathBuf::from("/dl"));
    let on_disk = |p: &Path| p == Path::new("/dl/a.txt");
    assert_eq!(
        reservations.reserve(Path::new("a.txt"), dir, on_disk),
        Some(PathBuf::from("/dl/a (1).txt"))
    );
}

#[test]
fn http_url_argument_must_be_web() {
    assert!(parse_http_url("https://example.com/a", "get").is_ok());
    assert!(parse_http_url("file:///etc/passwd", "get").is_err());
    assert!(parse_http_url("example.com", "get").is_err());
}
