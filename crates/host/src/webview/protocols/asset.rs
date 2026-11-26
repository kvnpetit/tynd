//! `asset://localhost/<encoded absolute path>`: streams a local file the app
//! allow-listed in `assets.scope` (`convertFileSrc` on the client).
//!
//! Security:
//! - off unless the backend config declares `assets.scope`; an empty scope
//!   denies everything;
//! - the path is percent-decoded once and must be absolute, without `..`
//!   segments, NUL, UNC / device prefixes or (Windows) a `:` past the drive;
//! - the file is kernel-resolved (symlinks followed) and the resolved path must
//!   match `assets.scope` and not match `security.fs.deny`;
//! - only app-local pages may load it, and only their origin is echoed in
//!   `Access-Control-Allow-Origin`.
//!
//! Files are never read whole: a `Range` request reads at most
//! [`MAX_RANGE_BYTES`] at its offset, and a request without `Range` is served
//! whole only up to [`MAX_FULL_BYTES`] (media elements always send `Range`).

use std::{
    borrow::Cow,
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::{Component, Path, PathBuf},
    sync::{Arc, OnceLock},
};

use tynd_security_policy::Scope;
use wry::{
    http::{header, HeaderName, HeaderValue, Method, Request, Response, StatusCode},
    RequestAsyncResponder,
};

use super::{
    forbidden, preflight, range, range::ByteRange, request_access, text_response, with_cors,
    ProtocolResponse,
};

/// Largest slice one `Range` response reads; a longer range is answered with
/// its first `MAX_RANGE_BYTES` (the `Content-Range` says so and the media
/// stack requests the rest).
pub(super) const MAX_RANGE_BYTES: u64 = 4 * 1024 * 1024;
/// Largest file served to a request without `Range`.
pub(super) const MAX_FULL_BYTES: u64 = 32 * 1024 * 1024;

const EXPOSED_HEADERS: &str = "Accept-Ranges, Content-Length, Content-Range, Content-Type";

static SCOPE: OnceLock<Scope> = OnceLock::new();

/// Compile the startup `assets.scope`. Patterns are globs over absolute paths
/// (`/home/me/Videos/**`, `C:/Users/me/Videos/**`).
pub(super) fn configure(patterns: &[String]) -> Result<(), String> {
    let scope = compile_scope(patterns)?;
    SCOPE
        .set(scope)
        .map_err(|_| "assets: the scope is already configured".to_owned())
}

fn compile_scope(patterns: &[String]) -> Result<Scope, String> {
    for pattern in patterns {
        if !is_absolute_pattern(pattern) {
            return Err(format!(
                "assets.scope: '{pattern}' must be an absolute path glob such as \
                 '/home/me/Videos/**' or 'C:/Users/me/Videos/**'"
            ));
        }
    }
    Scope::try_new(patterns.to_vec(), Vec::new()).map_err(|error| format!("assets.scope: {error}"))
}

fn is_absolute_pattern(pattern: &str) -> bool {
    if cfg!(windows) {
        let bytes = pattern.as_bytes();
        bytes.len() >= 3
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && matches!(bytes[2], b'/' | b'\\')
    } else {
        pattern.starts_with('/')
    }
}

pub(super) fn enabled() -> bool {
    SCOPE.get().is_some()
}

/// Async entry point: checks the caller on the UI thread, then reads the file
/// on the call pool.
pub(super) fn handle(webview: &str, request: Request<Vec<u8>>, responder: RequestAsyncResponder) {
    let Some(scope) = SCOPE.get() else {
        responder.respond(text_response(
            StatusCode::NOT_FOUND,
            "asset protocol is disabled",
        ));
        return;
    };
    let Ok(cors) = request_access(&request, webview) else {
        responder.respond(forbidden());
        return;
    };
    if request.method() == Method::OPTIONS {
        responder.respond(preflight(&request, cors.as_deref(), "GET, HEAD, OPTIONS"));
        return;
    }
    let pending = Arc::new(parking_lot::Mutex::new(Some((request, responder))));
    let worker = Arc::clone(&pending);
    let submitted = crate::ipc::call_pool::submit(move || {
        if let Some((request, responder)) = worker.lock().take() {
            let response = serve(
                scope,
                &request,
                crate::security::policy::check_fs_not_denied,
            );
            responder.respond(with_cors(response, cors.as_deref(), EXPOSED_HEADERS));
        }
    });
    if let Err(error) = submitted {
        if let Some((_, responder)) = pending.lock().take() {
            responder.respond(text_response(
                StatusCode::SERVICE_UNAVAILABLE,
                format!("asset worker unavailable: {error}"),
            ));
        }
    }
}

/// Serve one request against `scope`. `fs_deny` is the `security.fs.deny`
/// check (a parameter so tests do not depend on the global policy).
fn serve(
    scope: &Scope,
    request: &Request<Vec<u8>>,
    fs_deny: impl Fn(&Path) -> Result<(), String>,
) -> ProtocolResponse {
    let head = request.method() == Method::HEAD;
    if !head && request.method() != Method::GET {
        return text_response(
            StatusCode::METHOD_NOT_ALLOWED,
            "the asset protocol only answers GET and HEAD",
        );
    }
    let Some(path) = decode_request_path(request.uri().path()) else {
        return text_response(
            StatusCode::BAD_REQUEST,
            "asset URL must carry an absolute file path (build it with convertFileSrc)",
        );
    };
    let resolved = match authorize(scope, &path, fs_deny) {
        Ok(resolved) => resolved,
        Err(Denied::Missing) => return text_response(StatusCode::NOT_FOUND, "asset not found"),
        Err(Denied::Scope(reason)) => {
            crate::tynd_warn!("asset request refused: {reason}");
            return text_response(StatusCode::FORBIDDEN, reason);
        },
    };
    let range_header = request
        .headers()
        .get(header::RANGE)
        .and_then(|value| value.to_str().ok());
    match read_asset(&resolved, range_header, head) {
        Ok(response) => response,
        Err(error) => {
            crate::tynd_warn!("asset read failed for '{}': {error}", resolved.display());
            text_response(StatusCode::NOT_FOUND, "asset not readable")
        },
    }
}

/// Decode the URL path into the absolute file path `convertFileSrc` encoded.
/// `None` for anything that is not a plain absolute path.
fn decode_request_path(uri_path: &str) -> Option<PathBuf> {
    let raw = uri_path.strip_prefix('/').unwrap_or(uri_path);
    let decoded = crate::webview::scheme::percent_decode_once(raw)?;
    if decoded.is_empty() || decoded.contains('\0') {
        return None;
    }
    if cfg!(windows) {
        // UNC (`\\server\share`, which would leak NTLM credentials), device
        // (`\\?\`, `\\.\`) and alternate-data-stream (`file:stream`) spellings.
        if decoded.starts_with(['\\', '/'])
            || decoded.get(2..).is_some_and(|rest| rest.contains(':'))
        {
            return None;
        }
    } else if !decoded.starts_with('/') {
        return None;
    }
    let path = PathBuf::from(decoded);
    let traversal = path
        .components()
        .any(|component| matches!(component, Component::ParentDir));
    (path.is_absolute() && !traversal).then_some(path)
}

#[derive(Debug, PartialEq, Eq)]
enum Denied {
    /// No such file (or not a regular file).
    Missing,
    /// Outside the scope or denied by the fs policy.
    Scope(String),
}

/// Resolve `path` through the kernel (symlinks followed) and check the result.
fn authorize(
    scope: &Scope,
    path: &Path,
    fs_deny: impl Fn(&Path) -> Result<(), String>,
) -> Result<PathBuf, Denied> {
    let resolved = dunce::canonicalize(path).map_err(|_| Denied::Missing)?;
    if !resolved.is_file() {
        return Err(Denied::Missing);
    }
    let text = resolved
        .to_str()
        .ok_or_else(|| Denied::Scope("asset path is not valid UTF-8".to_owned()))?;
    scope
        .check_fs_allow_list(text, "assets")
        .map_err(|error| Denied::Scope(format!("{error} (add it to assets.scope)")))?;
    fs_deny(&resolved).map_err(Denied::Scope)?;
    Ok(resolved)
}

/// Read the requested slice of `path` and build the 200 / 206 / 416 reply.
fn read_asset(
    path: &Path,
    range_header: Option<&str>,
    head: bool,
) -> std::io::Result<ProtocolResponse> {
    let mut file = File::open(path)?;
    let len = file.metadata()?.len();
    let mime = asset_mime(path, &mut file)?;
    let (status, start, end) = match range::parse(range_header, len) {
        ByteRange::Unsatisfiable => {
            let mut response =
                text_response(StatusCode::RANGE_NOT_SATISFIABLE, "range not satisfiable");
            insert(
                &mut response,
                header::CONTENT_RANGE,
                &format!("bytes */{len}"),
            );
            insert(&mut response, header::ACCEPT_RANGES, "bytes");
            return Ok(response);
        },
        ByteRange::Full if len > MAX_FULL_BYTES => {
            let mut response = text_response(
                StatusCode::PAYLOAD_TOO_LARGE,
                format!(
                    "asset is larger than {} MiB: request it with a Range header",
                    MAX_FULL_BYTES / (1024 * 1024)
                ),
            );
            insert(&mut response, header::ACCEPT_RANGES, "bytes");
            return Ok(response);
        },
        ByteRange::Full => (StatusCode::OK, 0, len),
        ByteRange::Partial { start, end } => {
            let end = end.min(start + MAX_RANGE_BYTES - 1);
            (StatusCode::PARTIAL_CONTENT, start, end + 1)
        },
    };
    let count = end - start;
    let body = if head || count == 0 {
        Vec::new()
    } else {
        file.seek(SeekFrom::Start(start))?;
        let mut body = Vec::with_capacity(usize::try_from(count).unwrap_or(0));
        file.take(count).read_to_end(&mut body)?;
        body
    };
    let mut response = Response::new(Cow::Owned(body));
    *response.status_mut() = status;
    insert(&mut response, header::CONTENT_TYPE, &mime);
    insert(&mut response, header::CONTENT_LENGTH, &count.to_string());
    insert(&mut response, header::ACCEPT_RANGES, "bytes");
    if status == StatusCode::PARTIAL_CONTENT {
        insert(
            &mut response,
            header::CONTENT_RANGE,
            &format!("bytes {start}-{}/{len}", end - 1),
        );
    }
    insert(&mut response, header::X_CONTENT_TYPE_OPTIONS, "nosniff");
    insert(&mut response, header::CACHE_CONTROL, "no-cache");
    // A file opened as a document (an `.html` / `.svg` asset) runs sandboxed,
    // without scripts or an origin.
    insert(
        &mut response,
        header::CONTENT_SECURITY_POLICY,
        "default-src 'none'; img-src asset: http://asset.localhost; media-src asset: \
         http://asset.localhost; style-src 'unsafe-inline'; sandbox",
    );
    insert(
        &mut response,
        HeaderName::from_static("cross-origin-resource-policy"),
        "cross-origin",
    );
    Ok(response)
}

fn insert(response: &mut ProtocolResponse, name: HeaderName, value: &str) {
    if let Ok(value) = HeaderValue::from_str(value) {
        response.headers_mut().insert(name, value);
    }
}

/// MIME type from the extension, then from the first bytes when the
/// extension is unknown. Compressed files keep `application/octet-stream`:
/// the protocol never sets `Content-Encoding`.
fn asset_mime(path: &Path, file: &mut File) -> std::io::Result<String> {
    let ext = path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_ascii_lowercase);
    if let Some(mime) = ext.as_deref().and_then(media_mime) {
        return Ok(mime.to_owned());
    }
    let by_ext = crate::util::mime_for_path(path);
    if by_ext != "application/octet-stream" {
        return Ok(match by_ext {
            "text/html" | "text/css" | "text/plain" | "application/javascript" => {
                format!("{by_ext}; charset=utf-8")
            },
            other => other.to_owned(),
        });
    }
    let mut sample = Vec::with_capacity(512);
    file.by_ref().take(512).read_to_end(&mut sample)?;
    file.seek(SeekFrom::Start(0))?;
    Ok(crate::util::mime_sniff(&sample)
        .unwrap_or("application/octet-stream")
        .to_owned())
}

/// Media and document types `util::mime_for_path` does not know, plus the
/// compressed suffixes it would otherwise strip.
fn media_mime(ext: &str) -> Option<&'static str> {
    Some(match ext {
        "gz" | "zst" | "zstd" | "br" => "application/octet-stream",
        "mov" => "video/quicktime",
        "m4v" => "video/mp4",
        "mkv" => "video/x-matroska",
        "ogv" => "video/ogg",
        "ts" | "m2ts" => "video/mp2t",
        "m4a" => "audio/mp4",
        "aac" => "audio/aac",
        "flac" => "audio/flac",
        "opus" | "oga" => "audio/ogg",
        "weba" => "audio/webm",
        "m3u8" => "application/vnd.apple.mpegurl",
        "mpd" => "application/dash+xml",
        "vtt" => "text/vtt; charset=utf-8",
        "srt" => "application/x-subrip",
        "csv" => "text/csv; charset=utf-8",
        "tif" | "tiff" => "image/tiff",
        "heic" => "image/heic",
        "jxl" => "image/jxl",
        _ => return None,
    })
}

#[cfg(test)]
mod tests;
