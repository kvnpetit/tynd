use std::{
    borrow::Cow,
    collections::HashMap,
    io::Read as _,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, OnceLock,
    },
};

use wry::http::{Request, Response, StatusCode};

/// When set, `handle` bypasses the static asset cache and reads from
/// disk on every request. Enables `tynd dev` to serve fresh bytes after
/// an editor save without restarting the host. Flipped to `true` by the
/// runtime binaries when they're launched with `--debug`.
static DEV_MODE: AtomicBool = AtomicBool::new(false);

/// Toggle dev-mode asset serving. No-op when the value is unchanged.
pub fn set_dev_mode(on: bool) {
    DEV_MODE.store(on, Ordering::Release);
}

fn dev_mode() -> bool {
    DEV_MODE.load(Ordering::Acquire)
}

struct Asset {
    /// Refcounted bytes — drop happens once no in-flight response holds a clone.
    bytes: Arc<[u8]>,
    /// MIME type for the asset, including charset where needed.
    mime: Box<str>,
    /// `Cache-Control` value — immutable for hashed assets, short for index.html.
    cache_control: &'static str,
}

struct AssetCache {
    map: HashMap<String, Asset>,
    /// The directory this cache was built from (for debug assertions).
    #[cfg(debug_assertions)]
    dir: String,
}

impl AssetCache {
    /// Load all files from `dir` into memory once. Uses an in-tree walker and
    /// fans out to `rayon` above `PARALLEL_THRESHOLD` (small trees stay
    /// sequential — task setup dominates).
    fn load(dir: &str) -> Result<Self, String> {
        let base = PathBuf::from(dir);

        let entries = crate::util::collect_files_recursive(&base)
            .map_err(|error| format!("failed to enumerate frontend directory '{dir}': {error}"))?;

        let processed: Vec<Result<(String, Asset), String>> = if entries.len() >= PARALLEL_THRESHOLD
        {
            use rayon::prelude::*;
            entries
                .par_iter()
                .map(|path| process_asset(&base, path))
                .collect()
        } else {
            entries
                .iter()
                .map(|path| process_asset(&base, path))
                .collect()
        };

        let mut map = HashMap::with_capacity(processed.len());
        let mut total_bytes = 0usize;
        for item in processed {
            let (key, asset) = item?;
            total_bytes = total_bytes
                .checked_add(asset.bytes.len())
                .ok_or_else(|| "frontend asset cache byte count overflowed".to_string())?;
            if total_bytes > MAX_ASSET_CACHE_BYTES {
                return Err(format!(
                    "frontend assets exceed the {MAX_ASSET_CACHE_BYTES}-byte cache cap"
                ));
            }
            if map.insert(key.clone(), asset).is_some() {
                return Err(format!(
                    "duplicate frontend asset key '{key}' (plain and compressed variants conflict)"
                ));
            }
        }

        Ok(Self {
            map,
            #[cfg(debug_assertions)]
            dir: dir.to_owned(),
        })
    }

    fn get(&self, path: &str) -> Option<&Asset> {
        self.map.get(path)
    }
}

/// Below this threshold we stay sequential — per-task rayon overhead would
/// dominate the actual decode cost.
const PARALLEL_THRESHOLD: usize = 32;
const MAX_ASSET_BYTES: u64 = 128 * 1024 * 1024;
const MAX_ASSET_CACHE_BYTES: usize = 512 * 1024 * 1024;

/// Read + optionally zstd-decompress one asset with hard memory ceilings.
fn process_asset(base: &PathBuf, path: &Path) -> Result<(String, Asset), String> {
    let metadata = std::fs::metadata(path)
        .map_err(|error| format!("failed to inspect asset '{}': {error}", path.display()))?;
    if metadata.len() > MAX_ASSET_BYTES {
        return Err(format!(
            "asset '{}' exceeds the {MAX_ASSET_BYTES}-byte input cap",
            path.display()
        ));
    }
    let bytes = std::fs::read(path)
        .map_err(|error| format!("failed to read asset '{}': {error}", path.display()))?;
    let rel = path
        .strip_prefix(base)
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .map_err(|error| format!("asset '{}' escaped frontend root: {error}", path.display()))?;

    // Decompress .zst in-memory — wry's custom protocol doesn't honour
    // Content-Encoding, so we serve the bytes uncompressed.
    let (key, bytes) = if rel
        .get(rel.len().saturating_sub(4)..)
        .is_some_and(|suffix| suffix.eq_ignore_ascii_case(".zst"))
    {
        let out = decompress_asset(&rel, &bytes)?;
        (rel[..rel.len() - 4].to_owned(), out)
    } else {
        (rel, bytes)
    };

    let mime_path = PathBuf::from(&key);
    let mime = mime_for_bytes(&mime_path, &bytes);
    let cache = if key == "index.html" || mime.starts_with("text/html") {
        "no-cache"
    } else {
        "public, max-age=31536000, immutable"
    };

    let bytes: Arc<[u8]> = Arc::from(bytes.into_boxed_slice());
    Ok((
        key,
        Asset {
            bytes,
            mime: mime.into_boxed_str(),
            cache_control: cache,
        },
    ))
}

/// zstd-decompress the asset `rel`, refusing output over `MAX_ASSET_BYTES`.
fn decompress_asset(rel: &str, compressed: &[u8]) -> Result<Vec<u8>, String> {
    let decoder = zstd::Decoder::new(compressed)
        .map_err(|error| format!("failed to open compressed asset '{rel}': {error}"))?;
    let mut out = Vec::new();
    decoder
        .take(MAX_ASSET_BYTES + 1)
        .read_to_end(&mut out)
        .map_err(|error| format!("failed to decompress asset '{rel}': {error}"))?;
    if out.len() as u64 > MAX_ASSET_BYTES {
        return Err(format!(
            "decompressed asset '{rel}' exceeds the {MAX_ASSET_BYTES}-byte cap"
        ));
    }
    Ok(out)
}

static CACHE: OnceLock<Result<AssetCache, String>> = OnceLock::new();

fn get_cache(static_dir: &str) -> Result<&'static AssetCache, &'static str> {
    let cache = CACHE.get_or_init(|| AssetCache::load(static_dir));
    // Debug-only drift guard — `dir` is stripped in release.
    #[cfg(debug_assertions)]
    if cache.as_ref().is_ok_and(|cache| cache.dir != static_dir) {
        eprintln!(
            "[tynd] FATAL: scheme cache mismatch — initialized with '{}', called with \
             '{static_dir}'",
            cache
                .as_ref()
                .map_or("<failed>", |cache| cache.dir.as_str())
        );
        std::process::abort();
    }
    let _ = static_dir; // keep the param non-warning when the cfg is off
    cache.as_ref().map_err(String::as_str)
}

/// Pre-warm the asset cache in the calling thread.
/// Call this from a background thread at startup so the first request is instant.
pub(crate) fn warm(static_dir: &str) -> Result<(), String> {
    get_cache(static_dir).map(|_| ()).map_err(str::to_owned)
}

/// Serve static files via the `tynd://` custom protocol.
/// All files are loaded into memory on first request — subsequent requests
/// are served from memory with zero disk I/O. In dev mode (`set_dev_mode(true)`)
/// the cache is bypassed so editor saves land on the next webview reload.
pub(crate) fn handle(static_dir: &str, request: &Request<Vec<u8>>) -> Response<Cow<'static, [u8]>> {
    let Some(path) = decode_asset_path(request.uri().path()) else {
        return not_found();
    };
    let path = path.as_str();

    if dev_mode() {
        return serve_from_disk(static_dir, path);
    }

    let cache = match get_cache(static_dir) {
        Ok(cache) => cache,
        Err(error) => {
            crate::tynd_error!("frontend asset cache unavailable: {error}");
            return internal_error();
        },
    };
    let asset = cache.get(path).or_else(|| {
        // Only fall back for extensionless paths (HTML routes), not missing assets
        if has_file_extension(path) {
            None
        } else {
            cache.get("index.html")
        }
    });

    match asset {
        // wry needs `Cow<'static, [u8]>`, which cannot borrow from the
        // cache's `Arc`, so every response copies the asset bytes (a very
        // large asset doubles its peak memory while in flight; the cache
        // itself is capped by `MAX_ASSET_CACHE_BYTES`).
        Some(a) => asset_response(&a.mime, a.cache_control, a.bytes.to_vec()),
        None => not_found(),
    }
}

/// 200 response carrying one asset.
///
/// Every asset refuses MIME-sniffing so a misconfigured Content-Type can't be
/// silently re-interpreted by the webview as e.g. `text/html` and turn an
/// asset into a script-execution surface. HTML additionally gets the
/// cross-origin isolation headers and the baseline CSP.
fn asset_response(mime: &str, cache_control: &str, body: Vec<u8>) -> Response<Cow<'static, [u8]>> {
    let mut rsp = Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", mime)
        .header("X-Content-Type-Options", "nosniff")
        .header("Cache-Control", cache_control);
    if mime.starts_with("text/html") {
        rsp = rsp
            .header("Cross-Origin-Opener-Policy", "same-origin")
            .header("Cross-Origin-Embedder-Policy", "credentialless")
            .header("Content-Security-Policy", default_csp());
    }
    // `body` only errors on an invalid header value: the MIME comes from
    // `mime_for_bytes` and every other value is a static literal.
    rsp.body(Cow::Owned(body))
        .expect("scheme: 200 response body — header values are always valid")
}

/// Percent-decode a request path into a relative asset key (no leading `/`).
///
/// Returns `None` for anything that could escape the asset root or is not a
/// valid file name: invalid UTF-8, NUL, backslashes, `..` segments (also when
/// spelled `%2e%2e`), and on Windows `:` (drive / alternate-data-stream
/// syntax). Decoding happens exactly once, so `%252e` stays a literal `%2e`.
fn decode_asset_path(raw: &str) -> Option<String> {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hi = hex_value(*bytes.get(i + 1)?)?;
            let lo = hex_value(*bytes.get(i + 2)?)?;
            out.push(hi << 4 | lo);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    let decoded = String::from_utf8(out).ok()?;
    if decoded.contains(['\0', '\\']) {
        return None;
    }
    let trimmed = decoded.trim_start_matches('/');
    if trimmed
        .split('/')
        .any(|segment| segment == ".." || (cfg!(windows) && segment.contains(':')))
    {
        return None;
    }
    Some(trimmed.to_owned())
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// Whether the last path segment looks like a file name (`app.js`). Only the
/// last segment counts, so client routes like `/docs/v1.2/intro` and
/// `/files/` still fall back to the SPA `index.html`.
fn has_file_extension(path: &str) -> bool {
    path.rsplit('/')
        .next()
        .is_some_and(|segment| segment.contains('.'))
}

/// Resolve a decoded request path to a file under `base` for dev mode.
///
/// A directory (including the root, `""`) serves its `index.html`, falling back
/// to the SPA index; a missing extensionless path falls back to the SPA index;
/// anything that resolves outside `base` is refused.
fn resolve_dev_target(base: &Path, path: &str) -> Option<PathBuf> {
    // `dunce` strips Windows `\\?\` UNC prefixes so `starts_with(base)` works
    // — `std::fs::canonicalize` leaves them, breaking the traversal guard.
    let base_canon = dunce::canonicalize(base).unwrap_or_else(|_| base.to_path_buf());
    let spa_index = || {
        dunce::canonicalize(base.join("index.html"))
            .ok()
            .filter(|index| index.starts_with(&base_canon))
    };
    match dunce::canonicalize(base.join(path)) {
        Ok(canon) if !canon.starts_with(&base_canon) => None,
        Ok(canon) if canon.is_dir() => dunce::canonicalize(canon.join("index.html"))
            .ok()
            .filter(|index| index.starts_with(&base_canon) && index.is_file())
            .or_else(spa_index),
        Ok(canon) => Some(canon),
        Err(_) if !has_file_extension(path) => spa_index(),
        Err(_) => None,
    }
}

/// Dev-mode asset reader — reads from `static_dir` on every request so
/// editor saves show up on the next reload. SPA fallback mirrors the
/// cached path: extensionless paths that miss fall back to `index.html`.
fn serve_from_disk(static_dir: &str, path: &str) -> Response<Cow<'static, [u8]>> {
    let Some(target) = resolve_dev_target(Path::new(static_dir), path) else {
        return not_found();
    };
    let Ok(bytes) = std::fs::read(&target) else {
        return not_found();
    };
    let mime = mime_for_bytes(&target, &bytes);
    // Dev: never cache in the webview — editor saves must land.
    asset_response(&mime, "no-store", bytes)
}

fn not_found() -> Response<Cow<'static, [u8]>> {
    Response::builder()
        .status(StatusCode::NOT_FOUND)
        .header("Content-Type", "text/plain")
        .header("X-Content-Type-Options", "nosniff")
        .body(Cow::Borrowed(b"404 Not Found" as &[u8]))
        .expect("scheme: 404 response body — all headers are static literals")
}

fn internal_error() -> Response<Cow<'static, [u8]>> {
    Response::builder()
        .status(StatusCode::INTERNAL_SERVER_ERROR)
        .header("Content-Type", "text/plain")
        .header("X-Content-Type-Options", "nosniff")
        .body(Cow::Borrowed(b"500 Frontend Assets Unavailable" as &[u8]))
        .expect("scheme: 500 response body uses static status and header values")
}

/// Baseline CSP for every HTML response served via `tynd://`. Apps that need
/// to override should set a `<meta http-equiv="Content-Security-Policy">`.
///
/// The `style-src 'unsafe-inline'` grant is deliberate — Vite's HMR
/// shim and Tailwind's JIT runtime both inject `<style>` tags whose
/// content is generated at build / dev time, with no stable hash to
/// pin via `'unsafe-hashes'`. Locking style-src to `'self' tynd:`
/// alone breaks every app on first page load. The `script-src` side
/// stays strict (`'self' tynd:` only) so the `<script>` half of the
/// XSS surface is unaffected.
fn default_csp() -> &'static str {
    "default-src 'self' tynd: tynd-bin:; img-src 'self' tynd: tynd-bin: data: blob:; media-src \
     'self' tynd: tynd-bin: data: blob:; script-src 'self' tynd:; style-src 'self' tynd: \
     'unsafe-inline'; font-src 'self' tynd: data:; connect-src 'self' tynd: tynd-bin: \
     http://tynd-bin.localhost ipc: https: wss:; worker-src 'self' tynd: blob:; frame-src 'none'; \
     object-src 'none'; base-uri 'self'; form-action 'self'"
}

/// MIME resolution with optional content sniffing.
///
/// Order: magic-byte sniff (skipped for known-text extensions) →
/// `mime_for` extension lookup → `application/octet-stream`.
fn mime_for_bytes(path: &Path, bytes: &[u8]) -> String {
    let ext = path.extension().and_then(|e| e.to_str());
    let always_text = matches!(
        ext,
        Some("html" | "htm" | "css" | "ts" | "tsx" | "mjs" | "cjs" | "js" | "json" | "txt" | "svg")
    );
    if !always_text {
        // 8 KiB sample is well above the signatures we check.
        let sample = &bytes[..bytes.len().min(8192)];
        if let Some(mime) = crate::util::mime_sniff(sample) {
            return mime.to_string();
        }
    }
    mime_for(path)
}

/// Path-only MIME resolution. Strips transport-encoded suffixes
/// (`.zst` / `.gz`) before lookup so `index.js.gz` still resolves as JS.
fn mime_for(path: &Path) -> String {
    let ext = path.extension().and_then(|e| e.to_str());
    let lookup_path = match ext {
        Some("gz" | "zst") => path
            .file_stem()
            .map_or_else(|| path.to_path_buf(), PathBuf::from),
        _ => path.to_path_buf(),
    };
    // Bundler artefacts (`.ts` / `.tsx` / `.mjs` / `.cjs`) serve as JS.
    let s = crate::util::mime_for_path(&lookup_path);
    if s == "application/javascript" {
        return "application/javascript; charset=utf-8".to_string();
    }
    // Tag only the text MIMEs that downstream pattern-match on the prefix.
    if matches!(s, "text/html" | "text/css") {
        format!("{s}; charset=utf-8")
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_csp_allows_blob_workers_and_the_windows_bin_origin() {
        let csp = default_csp();
        assert!(csp.contains("worker-src 'self' tynd: blob:"));
        assert!(csp.contains("http://tynd-bin.localhost"));
        // Scripts stay strict: no blob: in script-src.
        assert!(csp.contains("script-src 'self' tynd:;"));
        assert!(csp.contains("frame-src 'none'"));
    }

    #[test]
    fn decodes_percent_escapes_once() {
        assert_eq!(decode_asset_path("/").as_deref(), Some(""));
        assert_eq!(
            decode_asset_path("/index.html").as_deref(),
            Some("index.html")
        );
        assert_eq!(
            decode_asset_path("/fonts/Noto%20Sans.woff2").as_deref(),
            Some("fonts/Noto Sans.woff2")
        );
        assert_eq!(
            decode_asset_path("/caf%C3%A9.png").as_deref(),
            Some("café.png")
        );
        // Single decode only: %25 stays a literal percent sign.
        assert_eq!(decode_asset_path("/%252e%252e").as_deref(), Some("%2e%2e"));
        assert_eq!(decode_asset_path("/a..b/c").as_deref(), Some("a..b/c"));
    }

    #[test]
    fn rejects_traversal_and_malformed_paths() {
        for bad in [
            "/../secret",
            "/a/../../secret",
            "/%2e%2e/secret",
            "/%2E%2E/secret",
            "/a/%2e%2e%2fsecret",
            "/a%5c..%5csecret",
            "/a%5cb",
            "/a%00b",
            "/bad%zz",
            "/trunc%2",
            "/%ff%fe",
        ] {
            assert_eq!(decode_asset_path(bad), None, "{bad} must be refused");
        }
    }

    #[cfg(windows)]
    #[test]
    fn rejects_drive_and_stream_syntax_on_windows() {
        assert_eq!(decode_asset_path("/C:/Windows/win.ini"), None);
        assert_eq!(decode_asset_path("/index.html:stream"), None);
    }

    #[test]
    fn spa_fallback_looks_at_the_last_segment_only() {
        assert!(has_file_extension("app.js"));
        assert!(has_file_extension("assets/app.abc.js"));
        assert!(!has_file_extension(""));
        assert!(!has_file_extension("users"));
        assert!(!has_file_extension("docs/"));
        assert!(!has_file_extension("docs/v1.2/intro"));
        assert!(!has_file_extension("v1.2/"));
    }

    fn site() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("index.html"), "<html>root</html>").unwrap();
        std::fs::create_dir(dir.path().join("docs")).unwrap();
        std::fs::write(
            dir.path().join("docs").join("index.html"),
            "<html>docs</html>",
        )
        .unwrap();
        std::fs::create_dir(dir.path().join("empty")).unwrap();
        std::fs::write(dir.path().join("Noto Sans.woff2"), b"font").unwrap();
        dir
    }

    fn read(base: &Path, path: &str) -> Option<String> {
        resolve_dev_target(base, path).map(|file| std::fs::read_to_string(file).unwrap())
    }

    #[test]
    fn dev_root_and_directories_serve_index_html() {
        let dir = site();
        assert_eq!(read(dir.path(), "").as_deref(), Some("<html>root</html>"));
        assert_eq!(
            read(dir.path(), "docs").as_deref(),
            Some("<html>docs</html>")
        );
        assert_eq!(
            read(dir.path(), "docs/").as_deref(),
            Some("<html>docs</html>")
        );
        // A directory without an index falls back to the SPA index.
        assert_eq!(
            read(dir.path(), "empty").as_deref(),
            Some("<html>root</html>")
        );
    }

    #[test]
    fn dev_missing_routes_fall_back_but_missing_files_do_not() {
        let dir = site();
        assert_eq!(
            read(dir.path(), "settings/profile").as_deref(),
            Some("<html>root</html>")
        );
        assert_eq!(
            read(dir.path(), "docs/v1.2/intro").as_deref(),
            Some("<html>root</html>")
        );
        assert_eq!(read(dir.path(), "missing.js"), None);
        assert_eq!(read(dir.path(), "Noto Sans.woff2").as_deref(), Some("font"));
    }

    #[test]
    fn dev_target_never_escapes_the_root() {
        let dir = site();
        let outer = tempfile::tempdir().expect("tempdir");
        std::fs::write(outer.path().join("secret.txt"), "secret").unwrap();
        let base = dir.path().join("docs");
        assert_eq!(read(&base, "../index.html"), None);
        let escape = format!(
            "../../{}/secret.txt",
            outer.path().file_name().unwrap().to_string_lossy()
        );
        assert_eq!(read(&base, &escape), None);
        // The public entry point refuses the encoded spelling before any I/O.
        assert_eq!(decode_asset_path("/%2e%2e/secret.txt"), None);
    }
}
