use super::*;

/// URL path `convertFileSrc` builds: the whole path as one encoded segment.
fn url_path(path: &Path) -> String {
    let raw = path.to_str().unwrap();
    let encoded: String = form_urlencoded::byte_serialize(raw.as_bytes()).collect();
    // `byte_serialize` writes spaces as `+`; `encodeURIComponent` uses `%20`.
    format!("/{}", encoded.replace('+', "%20"))
}

fn request(method: &str, path: &str, range: Option<&str>) -> Request<Vec<u8>> {
    let mut builder = Request::builder()
        .method(method)
        .uri(format!("asset://localhost{path}"));
    if let Some(range) = range {
        builder = builder.header("range", range);
    }
    builder.body(Vec::new()).unwrap()
}

fn glob_for(dir: &Path) -> String {
    let canonical = dunce::canonicalize(dir).unwrap();
    format!("{}/**", canonical.to_str().unwrap().replace('\\', "/"))
}

fn allow_all(_: &Path) -> Result<(), String> {
    Ok(())
}

struct Fixture {
    _dir: tempfile::TempDir,
    root: PathBuf,
    scope: Scope,
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let root = dunce::canonicalize(dir.path()).unwrap();
    std::fs::create_dir_all(root.join("media")).unwrap();
    std::fs::create_dir_all(root.join("private")).unwrap();
    let body: Vec<u8> = (0..=255u8).cycle().take(1000).collect();
    std::fs::write(root.join("media").join("clip one.mp4"), &body).unwrap();
    std::fs::write(root.join("media").join("notes.txt"), b"hello").unwrap();
    std::fs::write(root.join("media").join("blob"), b"\x89PNG\r\n\x1a\nrest").unwrap();
    std::fs::write(root.join("private").join("secret.txt"), b"secret").unwrap();
    let scope = compile_scope(&[glob_for(&root.join("media"))]).unwrap();
    Fixture {
        _dir: dir,
        root,
        scope,
    }
}

#[test]
fn scope_patterns_must_be_absolute_globs() {
    assert!(compile_scope(&[]).is_ok());
    let relative = compile_scope(&["**/*.mp4".to_owned()]).unwrap_err();
    assert!(relative.contains("absolute path glob"), "{relative}");
    let absolute = if cfg!(windows) {
        "C:/Videos/**"
    } else {
        "/videos/**"
    };
    assert!(compile_scope(&[absolute.to_owned()]).is_ok());
    let invalid = if cfg!(windows) { "C:/[" } else { "/[" };
    assert!(compile_scope(&[invalid.to_owned()]).is_err());
}

#[test]
fn request_paths_decode_to_plain_absolute_paths() {
    if cfg!(windows) {
        assert_eq!(
            decode_request_path("/C%3A%5CVideos%5Ca%20b.mp4"),
            Some(PathBuf::from(r"C:\Videos\a b.mp4"))
        );
        assert_eq!(
            decode_request_path("/C:/Videos/a.mp4"),
            Some(PathBuf::from("C:/Videos/a.mp4"))
        );
        for bad in [
            "/%5C%5Cserver%5Cshare%5Ca.mp4",
            "//server/share/a.mp4",
            "/%5C%5C%3F%5CC%3A%5Ca.mp4",
            "/C%3A%5Ca.mp4%3Astream",
            "/Videos%5Ca.mp4",
        ] {
            assert_eq!(decode_request_path(bad), None, "{bad}");
        }
    } else {
        assert_eq!(
            decode_request_path("/%2Fhome%2Fme%2Fa%20b.mp4"),
            Some(PathBuf::from("/home/me/a b.mp4"))
        );
        assert_eq!(decode_request_path("/home%2Fme"), None);
    }
    for bad in [
        "/",
        "",
        "/%2",
        "/%zz",
        "/%FF",
        "/%00",
        "/%2Fa%2F..%2Fb",
        "/C%3A%5Ca%5C..%5Cb",
    ] {
        assert_eq!(decode_request_path(bad), None, "{bad}");
    }
}

#[test]
fn files_inside_the_scope_are_served_with_their_mime() {
    let fx = fixture();
    let clip = fx.root.join("media").join("clip one.mp4");
    let response = serve(
        &fx.scope,
        &request("GET", &url_path(&clip), None),
        allow_all,
    );
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["content-type"], "video/mp4");
    assert_eq!(response.headers()["content-length"], "1000");
    assert_eq!(response.headers()["accept-ranges"], "bytes");
    assert_eq!(response.headers()["x-content-type-options"], "nosniff");
    assert!(response.headers()["content-security-policy"]
        .to_str()
        .unwrap()
        .ends_with("sandbox"));
    assert_eq!(response.body().len(), 1000);

    let notes = fx.root.join("media").join("notes.txt");
    let response = serve(
        &fx.scope,
        &request("GET", &url_path(&notes), None),
        allow_all,
    );
    assert_eq!(
        response.headers()["content-type"],
        "text/plain; charset=utf-8"
    );

    let sniffed = fx.root.join("media").join("blob");
    let response = serve(
        &fx.scope,
        &request("GET", &url_path(&sniffed), None),
        allow_all,
    );
    assert_eq!(response.headers()["content-type"], "image/png");
    assert_eq!(response.body().as_ref(), b"\x89PNG\r\n\x1a\nrest");
}

#[test]
fn ranges_are_served_as_partial_content() {
    let fx = fixture();
    let clip = url_path(&fx.root.join("media").join("clip one.mp4"));
    let response = serve(
        &fx.scope,
        &request("GET", &clip, Some("bytes=10-19")),
        allow_all,
    );
    assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(response.headers()["content-range"], "bytes 10-19/1000");
    assert_eq!(response.headers()["content-length"], "10");
    let expected: Vec<u8> = (10..20).collect();
    assert_eq!(response.body().as_ref(), expected.as_slice());

    let tail = serve(
        &fx.scope,
        &request("GET", &clip, Some("bytes=-5")),
        allow_all,
    );
    assert_eq!(tail.headers()["content-range"], "bytes 995-999/1000");
    assert_eq!(tail.body().len(), 5);

    let past = serve(
        &fx.scope,
        &request("GET", &clip, Some("bytes=5000-")),
        allow_all,
    );
    assert_eq!(past.status(), StatusCode::RANGE_NOT_SATISFIABLE);
    assert_eq!(past.headers()["content-range"], "bytes */1000");

    let head = serve(
        &fx.scope,
        &request("HEAD", &clip, Some("bytes=0-")),
        allow_all,
    );
    assert_eq!(head.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(head.headers()["content-length"], "1000");
    assert!(head.body().is_empty());
}

#[test]
fn long_ranges_are_capped_and_large_full_reads_refused() {
    let dir = tempfile::tempdir().unwrap();
    let root = dunce::canonicalize(dir.path()).unwrap();
    let big = root.join("big.bin");
    let file = File::create(&big).unwrap();
    file.set_len(MAX_FULL_BYTES + 1).unwrap();
    drop(file);
    let scope = compile_scope(&[glob_for(&root)]).unwrap();
    let path = url_path(&big);

    let open = serve(&scope, &request("GET", &path, Some("bytes=0-")), allow_all);
    assert_eq!(open.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(
        open.headers()["content-range"],
        format!("bytes 0-{}/{}", MAX_RANGE_BYTES - 1, MAX_FULL_BYTES + 1).as_str()
    );
    assert_eq!(u64::try_from(open.body().len()).unwrap(), MAX_RANGE_BYTES);

    let full = serve(&scope, &request("GET", &path, None), allow_all);
    assert_eq!(full.status(), StatusCode::PAYLOAD_TOO_LARGE);
}

#[test]
fn paths_outside_the_scope_or_denied_by_fs_are_refused() {
    let fx = fixture();
    let secret = url_path(&fx.root.join("private").join("secret.txt"));
    let response = serve(&fx.scope, &request("GET", &secret, None), allow_all);
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert!(std::str::from_utf8(response.body())
        .unwrap()
        .contains("assets.scope"));

    let clip = url_path(&fx.root.join("media").join("clip one.mp4"));
    let denied = serve(&fx.scope, &request("GET", &clip, None), |_| {
        Err("fs: denied by security policy".to_owned())
    });
    assert_eq!(denied.status(), StatusCode::FORBIDDEN);

    let empty = compile_scope(&[]).unwrap();
    assert_eq!(
        serve(&empty, &request("GET", &clip, None), allow_all).status(),
        StatusCode::FORBIDDEN
    );
}

#[test]
fn traversal_missing_files_and_other_methods_are_refused() {
    let fx = fixture();
    let media = fx.root.join("media");
    let traversal = format!(
        "{}{}",
        url_path(&media),
        form_urlencoded::byte_serialize(
            format!("{0}..{0}private{0}secret.txt", std::path::MAIN_SEPARATOR).as_bytes()
        )
        .collect::<String>()
    );
    assert_eq!(
        serve(&fx.scope, &request("GET", &traversal, None), allow_all).status(),
        StatusCode::BAD_REQUEST
    );
    let missing = url_path(&media.join("missing.mp4"));
    assert_eq!(
        serve(&fx.scope, &request("GET", &missing, None), allow_all).status(),
        StatusCode::NOT_FOUND
    );
    // A directory is not an asset.
    assert_eq!(
        serve(
            &fx.scope,
            &request("GET", &url_path(&media), None),
            allow_all
        )
        .status(),
        StatusCode::NOT_FOUND
    );
    let clip = url_path(&media.join("clip one.mp4"));
    assert_eq!(
        serve(&fx.scope, &request("POST", &clip, None), allow_all).status(),
        StatusCode::METHOD_NOT_ALLOWED
    );
}

#[cfg(unix)]
#[test]
fn symlinks_are_checked_at_their_target() {
    let fx = fixture();
    let link = fx.root.join("media").join("link.txt");
    std::os::unix::fs::symlink(fx.root.join("private").join("secret.txt"), &link).unwrap();
    assert_eq!(
        serve(
            &fx.scope,
            &request("GET", &url_path(&link), None),
            allow_all
        )
        .status(),
        StatusCode::FORBIDDEN
    );
}

#[test]
fn media_types_cover_common_containers() {
    assert_eq!(media_mime("mov"), Some("video/quicktime"));
    assert_eq!(media_mime("m4a"), Some("audio/mp4"));
    assert_eq!(media_mime("flac"), Some("audio/flac"));
    assert_eq!(media_mime("gz"), Some("application/octet-stream"));
    assert_eq!(media_mime("mp4"), None);
}
