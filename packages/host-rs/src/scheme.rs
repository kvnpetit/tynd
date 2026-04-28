use std::{
    borrow::Cow,
    collections::HashMap,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        OnceLock,
    },
};
use wry::http::{Request, Response, StatusCode};

/// When set, `handle` bypasses the static asset cache and reads from
/// disk on every request. Enables `tynd dev` to serve fresh bytes after
/// an editor save without restarting the host. Flipped to `true` by the
/// lite/full binaries when they're launched with `--debug`.
static DEV_MODE: AtomicBool = AtomicBool::new(false);

/// Toggle dev-mode asset serving. No-op when the value is unchanged.
pub fn set_dev_mode(on: bool) {
    DEV_MODE.store(on, Ordering::Release);
}

fn dev_mode() -> bool {
    DEV_MODE.load(Ordering::Acquire)
}

struct Asset {
    bytes: &'static [u8],
    /// MIME stored as a heap string so `mime_guess` can return arbitrary
    /// content types — we used to keep this `&'static str` against a
    /// hand-rolled match, but `mime_guess::Mime::essence_str` borrows from
    /// the per-extension table which the compiler can't see as `'static`.
    /// Boxing once at cache-load is free; the response builder copies.
    mime: Box<str>,
    /// `Cache-Control` value — immutable for hashed assets, short for index.html
    cache_control: &'static str,
}

struct AssetCache {
    map: HashMap<String, Asset>,
    /// The directory this cache was built from (for debug assertions).
    #[cfg(debug_assertions)]
    dir: String,
}

impl AssetCache {
    /// Load all files from `dir` into memory once.
    fn load(dir: &str) -> Self {
        let base = PathBuf::from(dir);
        let mut map = HashMap::new();
        walk(&base, &base, &mut map);
        Self {
            map,
            #[cfg(debug_assertions)]
            dir: dir.to_owned(),
        }
    }

    fn get(&self, path: &str) -> Option<&Asset> {
        self.map.get(path)
    }
}

#[allow(clippy::case_sensitive_file_extension_comparisons)] // .zst suffix written by CLI, always lowercase
fn walk(base: &PathBuf, cur: &PathBuf, map: &mut HashMap<String, Asset>) {
    let Ok(entries) = std::fs::read_dir(cur) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk(base, &path, map);
            continue;
        }
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        let rel = path
            .strip_prefix(base)
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .unwrap_or_default();

        // Detect zstd: strip .zst suffix, decompress bytes in memory.
        // Compression is only for binary size — assets are served uncompressed
        // because wry's custom protocol does not process Content-Encoding.
        let (key, bytes) = if rel.ends_with(".zst") {
            let key = rel[..rel.len() - 4].to_owned();
            match zstd::decode_all(bytes.as_slice()) {
                Ok(out) => (key, out),
                Err(e) => {
                    crate::tynd_warn!("Failed to decompress asset '{rel}': {e}");
                    continue;
                },
            }
        } else {
            (rel.clone(), bytes)
        };

        let mime_path = PathBuf::from(&key);
        let mime = mime_for(&mime_path);
        let cache = if key == "index.html" || mime.starts_with("text/html") {
            "no-cache"
        } else {
            "public, max-age=31536000, immutable"
        };

        let bytes: &'static [u8] = Box::leak(bytes.into_boxed_slice());
        map.insert(
            key,
            Asset {
                bytes,
                mime: mime.into_boxed_str(),
                cache_control: cache,
            },
        );
    }
}

static CACHE: OnceLock<AssetCache> = OnceLock::new();

fn get_cache(static_dir: &str) -> &'static AssetCache {
    let cache = CACHE.get_or_init(|| AssetCache::load(static_dir));
    // Drift guard. The `dir` field is only carried in debug builds (release
    // strips it for the few bytes saved on a process-global), so this check
    // is debug-only too. A regression here would be caught by the dev
    // workflow long before hitting a release runner.
    #[cfg(debug_assertions)]
    if cache.dir != static_dir {
        eprintln!(
            "[tynd] FATAL: scheme cache mismatch — initialized with '{}', \
             called with '{static_dir}'",
            cache.dir
        );
        std::process::abort();
    }
    let _ = static_dir; // keep the param non-warning when the cfg is off
    cache
}

/// Pre-warm the asset cache in the calling thread.
/// Call this from a background thread at startup so the first request is instant.
pub fn warm(static_dir: &str) {
    get_cache(static_dir);
}

/// Serve static files via the `tynd://` custom protocol.
/// All files are loaded into memory on first request — subsequent requests
/// are served from memory with zero disk I/O. In dev mode (`set_dev_mode(true)`)
/// the cache is bypassed so editor saves land on the next webview reload.
pub fn handle(static_dir: &str, request: &Request<Vec<u8>>) -> Response<Cow<'static, [u8]>> {
    let uri = request.uri().to_owned();
    let path = uri.path().trim_start_matches('/');

    if dev_mode() {
        return serve_from_disk(static_dir, path);
    }

    let cache = get_cache(static_dir);
    let asset = cache.get(path).or_else(|| {
        // Only fall back for extensionless paths (HTML routes), not missing assets
        let has_extension = path.contains('.') && !path.ends_with('/');
        if has_extension {
            None
        } else {
            cache.get("index.html")
        }
    });

    match asset {
        Some(a) => {
            let mut rsp = Response::builder()
                .status(StatusCode::OK)
                .header("Content-Type", a.mime.as_ref())
                .header("Cache-Control", a.cache_control);

            if a.mime.starts_with("text/html") {
                rsp = rsp
                    .header("Cross-Origin-Opener-Policy", "same-origin")
                    .header("Cross-Origin-Embedder-Policy", "credentialless")
                    .header("Content-Security-Policy", default_csp());
            }

            rsp.body(Cow::Borrowed(a.bytes)).unwrap()
        },
        None => Response::builder()
            .status(StatusCode::NOT_FOUND)
            .header("Content-Type", "text/plain")
            .body(Cow::Borrowed(b"404 Not Found" as &[u8]))
            .unwrap(),
    }
}

/// Dev-mode asset reader — reads from `static_dir` on every request so
/// editor saves show up on the next reload. SPA fallback mirrors the
/// cached path: extensionless paths that miss fall back to `index.html`.
fn serve_from_disk(static_dir: &str, path: &str) -> Response<Cow<'static, [u8]>> {
    let base = PathBuf::from(static_dir);
    let mut target = base.join(path);
    let has_extension = path.contains('.') && !path.ends_with('/');
    // `dunce::canonicalize` strips Windows UNC `\\?\` prefixes that
    // `std::fs::canonicalize` would otherwise leave on the result —
    // without that, a UNC `canon` could fail `starts_with(base)` even
    // when both resolve to the same drive path, breaking the dev-mode
    // path-traversal guard on Windows.
    if let Ok(canon) = dunce::canonicalize(&target) {
        let base_canon = dunce::canonicalize(&base).unwrap_or_else(|_| base.clone());
        if !canon.starts_with(&base_canon) {
            return not_found();
        }
        target = canon;
    } else if !has_extension {
        target = base.join("index.html");
    } else {
        return not_found();
    }
    let Ok(bytes) = std::fs::read(&target) else {
        return not_found();
    };
    let mime = mime_for(&target);
    let mut rsp = Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", &mime)
        // Dev: never cache in the webview — editor saves must land.
        .header("Cache-Control", "no-store");
    if mime.starts_with("text/html") {
        rsp = rsp
            .header("Cross-Origin-Opener-Policy", "same-origin")
            .header("Cross-Origin-Embedder-Policy", "credentialless")
            .header("Content-Security-Policy", default_csp());
    }
    rsp.body(Cow::Owned(bytes)).unwrap()
}

fn not_found() -> Response<Cow<'static, [u8]>> {
    Response::builder()
        .status(StatusCode::NOT_FOUND)
        .header("Content-Type", "text/plain")
        .body(Cow::Borrowed(b"404 Not Found" as &[u8]))
        .unwrap()
}

/// Baseline CSP applied to every HTML response served via `tynd://`. Blocks
/// inline/eval scripts by default; frames + objects refused entirely. Apps
/// that need third-party resources should migrate their `<script>`s to the
/// app bundle (where the default allows them) or set their own policy via
/// a `<meta http-equiv="Content-Security-Policy">` tag in `index.html`.
fn default_csp() -> &'static str {
    "default-src 'self' tynd: tynd-bin:; \
     img-src 'self' tynd: tynd-bin: data: blob:; \
     media-src 'self' tynd: tynd-bin: data: blob:; \
     script-src 'self' tynd:; \
     style-src 'self' tynd: 'unsafe-inline'; \
     font-src 'self' tynd: data:; \
     connect-src 'self' tynd: tynd-bin: ipc: https: wss:; \
     frame-src 'none'; \
     object-src 'none'; \
     base-uri 'self'; \
     form-action 'self'"
}

/// Resolve the response Content-Type for a static asset.
///
/// `mime_guess` carries the IANA registry, so we don't have to chase new
/// extensions whenever the frontend pulls in another asset family. We
/// strip transport-encoded suffixes (`.zst`, `.gz`) before lookup so an
/// `index.js.gz` still reports as `application/javascript` — wry serves
/// the bytes verbatim, the encoding header doesn't apply.
fn mime_for(path: &std::path::Path) -> String {
    let ext = path.extension().and_then(|e| e.to_str());
    let lookup_path = match ext {
        Some("gz" | "zst") => path
            .file_stem()
            .map_or_else(|| path.to_path_buf(), PathBuf::from),
        _ => path.to_path_buf(),
    };
    // `.ts` / `.tsx` / `.mjs` / `.cjs` are bundler artefacts that webview
    // serves as JavaScript. `mime_guess` would resolve `.ts` to
    // `video/mp2t`, so override these explicitly before falling back to
    // the registry.
    let final_ext = lookup_path.extension().and_then(|e| e.to_str());
    if matches!(final_ext, Some("ts" | "tsx" | "mjs" | "cjs" | "js")) {
        return "application/javascript; charset=utf-8".to_string();
    }
    let mime = mime_guess::from_path(&lookup_path).first_or_octet_stream();
    let s = mime.essence_str();
    // Append `; charset=utf-8` only for the text-shaped MIMEs the host
    // historically tagged that way — keeps the CSP + cache logic that
    // pattern-matches on `text/html` working byte-identically.
    if matches!(s, "text/html" | "text/css") {
        format!("{s}; charset=utf-8")
    } else {
        s.to_string()
    }
}
