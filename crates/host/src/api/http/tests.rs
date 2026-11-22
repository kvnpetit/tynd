//! `http.request` / `http.download` / `http.upload` against a one-shot
//! server on 127.0.0.1. The security policy stays unconfigured, so only the
//! scheme gate, size caps, timeouts and the cap-std FS confinement apply.

use std::{
    io::{BufRead as _, BufReader, Write as _},
    net::{TcpListener, TcpStream},
    path::Path,
    sync::mpsc,
    time::{Duration, Instant},
};

use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde_json::{json, Value};

use super::*;

struct Captured {
    head: String,
    body: Vec<u8>,
}

impl Captured {
    fn header(&self, name: &str) -> Option<String> {
        self.head.lines().skip(1).find_map(|line| {
            let (k, v) = line.split_once(':')?;
            k.trim()
                .eq_ignore_ascii_case(name)
                .then(|| v.trim().to_string())
        })
    }
}

fn read_request(stream: &mut TcpStream) -> Captured {
    let mut reader = BufReader::new(stream);
    let mut head = String::new();
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) == 0 {
            break;
        }
        head.push_str(&line);
        if line.trim_end_matches(['\r', '\n']).is_empty() {
            break;
        }
    }
    let mut captured = Captured {
        head,
        body: Vec::new(),
    };
    let chunked = captured
        .header("transfer-encoding")
        .is_some_and(|v| v.eq_ignore_ascii_case("chunked"));
    if chunked {
        loop {
            let mut size = String::new();
            if reader.read_line(&mut size).unwrap_or(0) == 0 {
                break;
            }
            let Ok(len) = usize::from_str_radix(size.trim(), 16) else {
                break;
            };
            let mut chunk = vec![0u8; len + 2]; // data + CRLF
            if reader.read_exact(&mut chunk).is_err() || len == 0 {
                break;
            }
            captured.body.extend_from_slice(&chunk[..len]);
        }
    } else {
        let len = captured
            .header("content-length")
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(0);
        let mut body = vec![0u8; len];
        if reader.read_exact(&mut body).is_ok() {
            captured.body = body;
        }
    }
    captured
}

/// Serve exactly one connection: capture the request, reply with `response`.
fn serve_once(response: Vec<u8>) -> (String, mpsc::Receiver<Captured>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let Ok((mut stream, _)) = listener.accept() else {
            return;
        };
        let captured = read_request(&mut stream);
        let _ = stream.write_all(&response);
        let _ = stream.flush();
        let _ = tx.send(captured);
        // Let the client finish reading before the socket closes.
        std::thread::sleep(Duration::from_millis(100));
    });
    (base, rx)
}

/// Accept one connection, send `prefix`, then stall for `stall`.
fn serve_stall(prefix: &'static [u8], stall: Duration) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    std::thread::spawn(move || {
        let Ok((mut stream, _)) = listener.accept() else {
            return;
        };
        let _ = read_request(&mut stream);
        let _ = stream.write_all(prefix);
        let _ = stream.flush();
        std::thread::sleep(stall);
    });
    base
}

fn ok_response(headers: &str, body: &[u8]) -> Vec<u8> {
    let mut out = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n{headers}\r\n",
        body.len()
    )
    .into_bytes();
    out.extend_from_slice(body);
    out
}

/// A port nothing listens on.
fn closed_port_url() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    format!("http://127.0.0.1:{port}/")
}

fn tmp() -> tempfile::TempDir {
    tempfile::tempdir().expect("tempdir")
}

fn p(base: &Path, rel: &str) -> String {
    base.join(rel).to_str().unwrap().to_string()
}

fn out_of_scope(file: &str) -> String {
    std::env::temp_dir()
        .ancestors()
        .last()
        .unwrap()
        .join("tynd-out-of-scope-test-dir")
        .join(file)
        .to_str()
        .unwrap()
        .to_string()
}

fn entries(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

// ---- scheme gate / dispatch -------------------------------------------

#[test]
fn url_scheme_gate() {
    for ok in ["http://a/", "https://a/", "ws://a/", "wss://a/"] {
        assert!(check_url_scheme(ok, "request").is_ok(), "{ok}");
    }
    for bad in [
        "file:///etc/passwd",
        "ftp://a/",
        "data:text/plain,x",
        "javascript:alert(1)",
    ] {
        let e = check_url_scheme(bad, "request").unwrap_err();
        assert!(e.starts_with("http.request: scheme '"), "{bad}: {e}");
    }
    let e = check_url_scheme("not a url", "download").unwrap_err();
    assert!(e.starts_with("http.download: invalid URL"), "{e}");
}

#[test]
fn dispatch_rejects_unknown_methods_and_bad_args() {
    let e = dispatch("nope", json!({})).unwrap_err();
    assert_eq!(e, "http.nope: unknown method");
    for method in ["request", "download", "upload"] {
        let e = dispatch(method, json!({})).unwrap_err();
        assert!(
            e.starts_with(&format!("http.{method}: invalid arguments")),
            "{e}"
        );
    }
    let e = dispatch("request", json!({ "url": "file:///etc/passwd" })).unwrap_err();
    assert!(e.contains("scheme 'file' is not allowed"), "{e}");
}

// ---- request ----------------------------------------------------------

#[test]
fn request_get_maps_status_headers_and_text_body() {
    let (base, rx) = serve_once(ok_response(
        "X-Custom: one\r\nSet-Cookie: a=1\r\nSet-Cookie: b=2\r\n",
        "héllo".as_bytes(),
    ));
    let out = dispatch(
        "request",
        json!({ "url": format!("{base}/path?q=1"), "headers": { "X-Req": "v", "X-Skip": 5 } }),
    )
    .unwrap();
    assert_eq!(out["status"], 200);
    assert_eq!(out["statusText"], "OK");
    assert_eq!(out["body"], "héllo");
    assert_eq!(
        out["headers"]["x-custom"], "one",
        "header names are lowercased"
    );
    assert_eq!(out["headers"]["set-cookie"], json!(["a=1", "b=2"]));

    let req = rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(
        req.head.starts_with("GET /path?q=1 HTTP/1.1"),
        "{}",
        req.head
    );
    assert_eq!(req.header("x-req").as_deref(), Some("v"));
    assert_eq!(
        req.header("x-skip"),
        None,
        "non-string header values are dropped"
    );
}

#[test]
fn request_json_body_defaults_content_type() {
    let (base, rx) = serve_once(ok_response("", b""));
    dispatch(
        "request",
        json!({ "url": base, "method": "POST", "body": { "a": 1 } }),
    )
    .unwrap();
    let req = rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(req.head.starts_with("POST "), "{}", req.head);
    assert_eq!(
        req.header("content-type").as_deref(),
        Some("application/json")
    );
    assert_eq!(req.body, br#"{"a":1}"#);
}

#[test]
fn request_json_body_keeps_caller_content_type() {
    let (base, rx) = serve_once(ok_response("", b""));
    dispatch(
        "request",
        json!({
            "url": base, "method": "PUT", "body": [1, 2],
            "headers": { "Content-Type": "application/vnd.x+json" },
        }),
    )
    .unwrap();
    let req = rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(
        req.header("content-type").as_deref(),
        Some("application/vnd.x+json")
    );
    assert_eq!(req.head.matches("application/json").count(), 0);
    assert_eq!(req.body, b"[1,2]");
}

#[test]
fn request_string_body_is_sent_verbatim_without_content_type() {
    let (base, rx) = serve_once(ok_response("", b""));
    dispatch(
        "request",
        json!({ "url": base, "method": "POST", "body": "raw=1&b=2" }),
    )
    .unwrap();
    let req = rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(req.body, b"raw=1&b=2");
    assert_eq!(req.header("content-type"), None);
}

#[test]
fn request_response_types() {
    let (base, _rx) = serve_once(ok_response("", br#"{"k":[1,true]}"#));
    let out = dispatch("request", json!({ "url": base, "responseType": "json" })).unwrap();
    assert_eq!(out["body"], json!({ "k": [1, true] }));

    let (base, _rx) = serve_once(ok_response("", &[0, 255, 10]));
    let out = dispatch("request", json!({ "url": base, "responseType": "binary" })).unwrap();
    assert_eq!(out["body"], STANDARD.encode([0, 255, 10]));

    let (base, _rx) = serve_once(ok_response("", b"<html>"));
    let e = dispatch("request", json!({ "url": base, "responseType": "json" })).unwrap_err();
    assert!(e.contains("response is not valid JSON"), "{e}");
}

#[test]
fn request_error_status_and_redirect_are_returned_not_followed() {
    let (base, _rx) = serve_once(
        b"HTTP/1.1 404 Not Found\r\nContent-Length: 4\r\nConnection: close\r\n\r\ngone".to_vec(),
    );
    let out = dispatch("request", json!({ "url": base })).unwrap();
    assert_eq!(out["status"], 404);
    assert_eq!(out["body"], "gone");

    let (base, _rx) = serve_once(
        b"HTTP/1.1 302 Found\r\nLocation: file:///etc/passwd\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            .to_vec(),
    );
    let out = dispatch("request", json!({ "url": base })).unwrap();
    assert_eq!(out["status"], 302);
    assert_eq!(out["headers"]["location"], "file:///etc/passwd");
}

#[test]
fn request_timeout_fires_on_stalled_server() {
    let base = serve_stall(b"", Duration::from_secs(10));
    let started = Instant::now();
    let e = dispatch("request", json!({ "url": base, "timeoutMs": 300 })).unwrap_err();
    assert!(e.starts_with("http.request: ("), "{e}");
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[test]
fn request_timeout_covers_stalled_body() {
    let base = serve_stall(
        b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\npartial",
        Duration::from_secs(10),
    );
    let started = Instant::now();
    let e = dispatch("request", json!({ "url": base, "timeoutMs": 1500 })).unwrap_err();
    assert!(e.contains("read body"), "{e}");
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[test]
fn request_connection_refused_is_an_error() {
    let e = dispatch("request", json!({ "url": closed_port_url() })).unwrap_err();
    assert!(e.starts_with("http.request: ("), "{e}");
}

#[test]
fn request_body_over_cap_is_refused_before_sending() {
    let body = "a".repeat(MAX_REQUEST_BODY + 1);
    // Nothing listens: the cap must trip before any connection attempt.
    let e = dispatch(
        "request",
        json!({ "url": closed_port_url(), "method": "POST", "body": body }),
    )
    .unwrap_err();
    assert!(e.contains("exceeds the"), "{e}");
}

#[test]
fn request_response_over_cap_is_refused() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    std::thread::spawn(move || {
        let Ok((mut stream, _)) = listener.accept() else {
            return;
        };
        let _ = read_request(&mut stream);
        let total = MAX_RESPONSE_BODY + 64 * 1024;
        let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: {total}\r\n\r\n");
        let chunk = vec![b'x'; 1024 * 1024];
        let mut sent = 0;
        while sent < total {
            let n = chunk.len().min(total - sent);
            if stream.write_all(&chunk[..n]).is_err() {
                return; // client hung up after the cap tripped
            }
            sent += n;
        }
    });
    let e = dispatch("request", json!({ "url": base, "responseType": "binary" })).unwrap_err();
    assert!(e.contains("response body exceeds"), "{e}");
}

// ---- download ---------------------------------------------------------

#[test]
fn download_streams_to_dest_creating_parents() {
    let dir = tmp();
    let dest = p(dir.path(), "a/b/file.bin");
    let (base, _rx) = serve_once(ok_response("", b"payload-bytes"));
    let out = dispatch("download", json!({ "url": base, "dest": dest })).unwrap();
    assert_eq!(out, json!({ "path": dest, "bytes": 13 }));
    assert_eq!(std::fs::read(&dest).unwrap(), b"payload-bytes");
    assert_eq!(
        entries(&dir.path().join("a/b")),
        ["file.bin"],
        "no temp leftovers"
    );
}

#[test]
fn download_overwrites_existing_dest_atomically() {
    let dir = tmp();
    let dest = p(dir.path(), "f");
    std::fs::write(&dest, "old content").unwrap();
    let (base, _rx) = serve_once(ok_response("", b"new"));
    dispatch("download", json!({ "url": base, "dest": dest })).unwrap();
    assert_eq!(std::fs::read(&dest).unwrap(), b"new");
}

#[test]
fn download_refuses_out_of_scope_dest_before_connecting() {
    let e = dispatch(
        "download",
        json!({ "url": closed_port_url(), "dest": out_of_scope("f") }),
    )
    .unwrap_err();
    assert!(e.contains("outside capability roots"), "{e}");
    let e = dispatch("download", json!({ "url": "file:///x", "dest": "f" })).unwrap_err();
    assert!(e.contains("scheme 'file'"), "{e}");
}

#[test]
fn download_refuses_declared_size_over_cap() {
    let dir = tmp();
    let dest = p(dir.path(), "sub/big.bin");
    let (base, _rx) = serve_once(
        b"HTTP/1.1 200 OK\r\nContent-Length: 20000000000\r\nConnection: close\r\n\r\n".to_vec(),
    );
    let e = dispatch("download", json!({ "url": base, "dest": dest })).unwrap_err();
    assert!(e.contains("exceeds the"), "{e}");
    assert!(entries(dir.path()).is_empty(), "nothing created on refusal");
}

#[test]
fn download_timeout_fails_on_stalled_body() {
    let dir = tmp();
    let dest = p(dir.path(), "f.bin");
    let base = serve_stall(
        b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\npartial",
        Duration::from_secs(10),
    );
    let started = Instant::now();
    let e = dispatch(
        "download",
        json!({ "url": base, "dest": dest, "timeoutMs": 1500 }),
    )
    .unwrap_err();
    assert!(e.starts_with("http.download: read"), "{e}");
    assert!(started.elapsed() < Duration::from_secs(5));
    assert!(
        !Path::new(&dest).exists(),
        "dest must not appear on failure"
    );
}

#[test]
fn download_failure_removes_temp_file() {
    let dir = tmp();
    let dest = p(dir.path(), "f.bin");
    let base = serve_stall(
        b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\npartial",
        Duration::from_secs(10),
    );
    let _ = dispatch(
        "download",
        json!({ "url": base, "dest": dest, "timeoutMs": 300 }),
    );
    assert!(
        entries(dir.path()).is_empty(),
        "left behind: {:?}",
        entries(dir.path())
    );
}

#[test]
fn download_non_2xx_status_fails_without_touching_dest() {
    let dir = tmp();
    let dest = p(dir.path(), "f.bin");
    std::fs::write(&dest, "old content").unwrap();
    let (base, _rx) = serve_once(
        b"HTTP/1.1 404 Not Found
Content-Length: 4
Connection: close

gone"
            .to_vec(),
    );
    let e = dispatch("download", json!({ "url": base, "dest": dest })).unwrap_err();
    assert!(e.starts_with("http.download: ("), "{e}");
    assert!(e.contains("HTTP status 404"), "{e}");
    assert_eq!(std::fs::read(&dest).unwrap(), b"old content");
    assert_eq!(entries(dir.path()), ["f.bin"], "no temp leftovers");

    let fresh = p(dir.path(), "new/g.bin");
    let (base, _rx) = serve_once(
        b"HTTP/1.1 500 Internal Server Error
Content-Length: 4
Connection: close

oops"
            .to_vec(),
    );
    let e = dispatch("download", json!({ "url": base, "dest": fresh })).unwrap_err();
    assert!(e.contains("HTTP status 500"), "{e}");
    assert!(!Path::new(&fresh).exists(), "dest must not be created");
}

// ---- upload -----------------------------------------------------------

#[test]
fn upload_sends_well_formed_multipart() {
    let dir = tmp();
    let file = p(dir.path(), "report.txt");
    std::fs::write(&file, "file-content").unwrap();
    let (base, rx) = serve_once(ok_response("", b"stored"));
    let out = dispatch(
        "upload",
        json!({ "url": base, "filePath": file, "fieldName": "doc", "headers": { "X-A": "1" } }),
    )
    .unwrap();
    assert_eq!(
        out,
        json!({ "status": 200, "body": STANDARD.encode(b"stored") })
    );

    let req = rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(req.head.starts_with("POST "), "{}", req.head);
    assert_eq!(req.header("x-a").as_deref(), Some("1"));
    let ct = req.header("content-type").unwrap();
    let boundary = ct
        .strip_prefix("multipart/form-data; boundary=")
        .unwrap_or_else(|| panic!("content-type: {ct}"));
    assert!(boundary.starts_with("----TyndFormBoundary"));
    assert_eq!(
        req.header("content-length")
            .unwrap()
            .parse::<usize>()
            .unwrap(),
        req.body.len(),
        "declared length matches the streamed body"
    );
    let body = String::from_utf8(req.body).unwrap();
    assert!(body.starts_with(&format!("--{boundary}\r\n")), "{body}");
    assert!(
        body.contains("Content-Disposition: form-data; name=\"doc\"; filename=\"report.txt\"\r\n"),
        "{body}"
    );
    assert!(body.contains("Content-Type: text/plain"), "{body}");
    assert!(
        body.ends_with(&format!("\r\n\r\nfile-content\r\n--{boundary}--\r\n")),
        "{body}"
    );
}

#[test]
fn upload_rejects_header_injection_in_field_name() {
    let dir = tmp();
    let file = p(dir.path(), "f.txt");
    std::fs::write(&file, "x").unwrap();
    for bad in ["a\r\nX-Evil: 1", "a\nb", "a\0b"] {
        let e = dispatch(
            "upload",
            json!({ "url": closed_port_url(), "filePath": file, "fieldName": bad }),
        )
        .unwrap_err();
        assert!(e.contains("forbidden control character"), "{e}");
    }
}

#[test]
fn multipart_params_are_escaped() {
    assert_eq!(
        upload::sanitize_multipart_param(r#"a"b\c"#).unwrap(),
        r#"a\"b\\c"#
    );
    assert_eq!(
        upload::sanitize_multipart_param("plain é").unwrap(),
        "plain é"
    );
    assert!(upload::sanitize_multipart_param("x\ry").is_err());
}

#[test]
fn multipart_boundaries_are_random() {
    let a = upload::make_boundary();
    let b = upload::make_boundary();
    assert_ne!(a, b);
    assert_eq!(a.len(), "----TyndFormBoundary".len() + 26);
}

#[test]
fn upload_refuses_missing_and_out_of_scope_files() {
    let dir = tmp();
    let e = dispatch(
        "upload",
        json!({ "url": closed_port_url(), "filePath": p(dir.path(), "missing.txt") }),
    )
    .unwrap_err();
    assert!(e.starts_with("http.upload: open"), "{e}");

    let e = dispatch(
        "upload",
        json!({ "url": closed_port_url(), "filePath": out_of_scope("f.txt") }),
    )
    .unwrap_err();
    assert!(e.contains("outside capability roots"), "{e}");
}

#[test]
fn upload_rejects_snake_case_field_names() {
    let dir = tmp();
    let file = p(dir.path(), "f.bin");
    std::fs::write(&file, [1u8, 2, 3]).unwrap();
    let e = dispatch(
        "upload",
        json!({ "url": closed_port_url(), "file_path": file, "field_name": "blob" }),
    )
    .unwrap_err();
    assert!(e.contains("filePath"), "{e}");
}

#[test]
fn json_args_values_are_forwarded_as_is() {
    // `headers` that is not an object is ignored rather than rejected.
    let (base, _rx) = serve_once(ok_response("", b"ok"));
    let out = dispatch("request", json!({ "url": base, "headers": "nope" })).unwrap();
    assert_eq!(out["body"], Value::String("ok".into()));
}
