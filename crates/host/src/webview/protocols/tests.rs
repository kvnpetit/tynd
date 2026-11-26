use super::*;

fn request_with_headers(headers: &[(&str, &str)]) -> Request<Vec<u8>> {
    let mut builder = Request::builder().uri("media://localhost/x");
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    builder.body(Vec::new()).unwrap()
}

#[test]
fn scheme_names_follow_dns_label_rules_and_skip_reserved_ones() {
    for ok in ["media", "my-app", "x1", "a"] {
        assert!(validate_scheme_name(ok).is_ok(), "{ok}");
    }
    for bad in [
        "",
        "Media",
        "1media",
        "-media",
        "media-",
        "me_dia",
        "me.dia",
        "me+dia",
        "médias",
        &"a".repeat(MAX_SCHEME_LEN + 1),
    ] {
        let error = validate_scheme_name(bad).unwrap_err();
        assert!(error.contains("is invalid"), "{bad}: {error}");
    }
    for reserved in ["tynd", "tynd-bin", "asset", "http", "https", "file", "blob"] {
        let error = validate_scheme_name(reserved).unwrap_err();
        assert!(error.contains("reserved"), "{reserved}: {error}");
    }
}

#[test]
fn csp_sources_cover_both_url_spellings() {
    assert_eq!(csp_sources_for(&[]), "");
    assert_eq!(
        csp_sources_for(&["asset", "media"]),
        "asset: http://asset.localhost media: http://media.localhost"
    );
}

#[test]
fn only_app_content_may_use_app_protocols() {
    let bundled = request_with_headers(&[("origin", "tynd://localhost")]);
    assert_eq!(
        request_access(&bundled, "main"),
        Ok(Some("tynd://localhost".to_owned()))
    );
    // `<img>` / `<video>` loads carry a Referer, not an Origin: no CORS echo.
    let referer = request_with_headers(&[("referer", "tynd://localhost/index.html")]);
    assert_eq!(request_access(&referer, "main"), Ok(None));
    let same_origin = request_with_headers(&[("sec-fetch-site", "same-origin")]);
    assert_eq!(request_access(&same_origin, "main"), Ok(None));

    for headers in [
        &[("origin", "https://evil.test")][..],
        &[("referer", "https://evil.test/page")][..],
        &[("sec-fetch-site", "cross-site")][..],
        &[][..],
    ] {
        let request = request_with_headers(headers);
        assert_eq!(
            request_access(&request, "main"),
            Err(Refused),
            "{headers:?}"
        );
    }
}

#[test]
fn cors_headers_name_only_the_app_origin() {
    let response = with_cors(
        text_response(StatusCode::OK, "x"),
        Some("tynd://localhost"),
        "Content-Range",
    );
    assert_eq!(
        response.headers()["access-control-allow-origin"],
        "tynd://localhost"
    );
    assert_eq!(
        response.headers()["access-control-expose-headers"],
        "Content-Range"
    );
    assert_eq!(response.headers()["vary"], "Origin");

    let no_cors = with_cors(text_response(StatusCode::OK, "x"), None, "Content-Range");
    assert!(no_cors
        .headers()
        .get("access-control-allow-origin")
        .is_none());
}

#[test]
fn preflights_answer_app_origins_only() {
    let request = request_with_headers(&[
        ("origin", "tynd://localhost"),
        ("access-control-request-headers", "range"),
    ]);
    let response = preflight(&request, Some("tynd://localhost"), "GET, HEAD, OPTIONS");
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert_eq!(
        response.headers()["access-control-allow-methods"],
        "GET, HEAD, OPTIONS"
    );
    assert_eq!(response.headers()["access-control-allow-headers"], "range");
    assert_eq!(
        preflight(&request, None, "GET").status(),
        StatusCode::FORBIDDEN
    );
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
