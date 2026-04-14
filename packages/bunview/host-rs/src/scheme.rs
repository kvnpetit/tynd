use std::{borrow::Cow, path::PathBuf};
use wry::http::{Request, Response, StatusCode};

/// Serve static files for the `bv://` custom protocol.
///
/// Maps `bv://localhost/<path>` → `<static_dir>/<path>`.
/// Falls back to `index.html` for SPA routing (unknown paths).
pub fn handle(
    static_dir: &str,
    request: Request<Vec<u8>>,
) -> Response<Cow<'static, [u8]>> {
    let uri  = request.uri().to_owned();
    let path = uri.path();

    let base       = PathBuf::from(static_dir);
    let rel        = path.trim_start_matches('/');
    let mut target = base.join(rel);

    // SPA routing: if the file doesn't exist or is a directory, serve index.html
    if !target.exists() || target.is_dir() {
        target = base.join("index.html");
    }

    match std::fs::read(&target) {
        Ok(bytes) => {
            let mime = mime_for(&target);
            let mut builder = Response::builder()
                .status(StatusCode::OK)
                .header("Content-Type", mime)
                .header("Access-Control-Allow-Origin", "*");

            // Cross-origin isolation: required for SharedArrayBuffer (WASM threads)
            // and full WebGPU access (Transformers.js, ONNX Runtime Web, etc.)
            //
            // COOP: same-origin  — isolates the browsing context
            // COEP: credentialless — enables isolation without blocking CDN assets
            //       (safer than "require-corp" which blocks all un-annotated cross-origin loads)
            if is_html(mime) {
                // Content-Security-Policy: tight default allowing own origin + wasm-unsafe-eval
                // for Transformers.js / ONNX WASM backend. `bv:` scheme is self-served here.
                // Users can override by shipping their own CSP meta tag (this header is weaker
                // than meta-CSP when both are present for the more permissive directive set).
                builder = builder
                    .header("Cross-Origin-Opener-Policy", "same-origin")
                    .header("Cross-Origin-Embedder-Policy", "credentialless")
                    .header(
                        "Content-Security-Policy",
                        "default-src 'self' bv: data: blob:; \
                         script-src 'self' bv: 'wasm-unsafe-eval'; \
                         style-src 'self' bv: 'unsafe-inline'; \
                         img-src 'self' bv: data: blob: https:; \
                         font-src 'self' bv: data:; \
                         connect-src 'self' bv: https: wss: ws:; \
                         worker-src 'self' bv: blob:;",
                    );
            }

            builder.body(Cow::Owned(bytes)).unwrap()
        }
        Err(_) => not_found(),
    }
}

fn not_found() -> Response<Cow<'static, [u8]>> {
    Response::builder()
        .status(StatusCode::NOT_FOUND)
        .header("Content-Type", "text/plain")
        .body(Cow::Borrowed(b"404 Not Found" as &[u8]))
        .unwrap()
}

fn is_html(mime: &str) -> bool {
    mime.starts_with("text/html")
}

fn mime_for(path: &std::path::Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()) {
        Some("html") | Some("htm") => "text/html; charset=utf-8",
        Some("js") | Some("mjs")   => "application/javascript; charset=utf-8",
        Some("css")                => "text/css; charset=utf-8",
        Some("json")               => "application/json",
        Some("png")                => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("gif")                => "image/gif",
        Some("svg")                => "image/svg+xml",
        Some("ico")                => "image/x-icon",
        Some("woff")               => "font/woff",
        Some("woff2")              => "font/woff2",
        Some("ttf")                => "font/ttf",
        Some("otf")                => "font/otf",
        Some("webp")               => "image/webp",
        Some("mp4")                => "video/mp4",
        Some("webm")               => "video/webm",
        Some("wasm")               => "application/wasm",
        _                          => "application/octet-stream",
    }
}
