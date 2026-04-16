use std::{
    borrow::Cow,
    collections::HashMap,
    io::Read,
    path::PathBuf,
    sync::OnceLock,
};
use flate2::read::GzDecoder;
use wry::http::{Request, Response, StatusCode};

// ── In-memory asset cache ─────────────────────────────────────────────────────

struct Asset {
    bytes:         &'static [u8],
    mime:          &'static str,
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

fn walk(base: &PathBuf, cur: &PathBuf, map: &mut HashMap<String, Asset>) {
    let Ok(entries) = std::fs::read_dir(cur) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk(base, &path, map);
            continue;
        }
        let Ok(bytes) = std::fs::read(&path) else { continue };
        let rel = path
            .strip_prefix(base)
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .unwrap_or_default();

        // Detect gzip: strip .gz suffix, decompress bytes in memory.
        // Compression is only for binary size — assets are served uncompressed
        // because wry's custom protocol does not process Content-Encoding.
        let (key, bytes) = if rel.ends_with(".gz") {
            let key = rel[..rel.len() - 3].to_owned();
            let mut dec = GzDecoder::new(bytes.as_slice());
            let mut out = Vec::new();
            if let Err(e) = dec.read_to_end(&mut out) {
                eprintln!("[vorn] Failed to decompress asset '{rel}': {e}");
                continue;
            }
            (key, out)
        } else {
            (rel.clone(), bytes)
        };

        let mime_path = PathBuf::from(&key);
        let mime  = mime_for(&mime_path);
        let cache = if key == "index.html" || mime.starts_with("text/html") {
            "no-cache"
        } else {
            "public, max-age=31536000, immutable"
        };

        let bytes: &'static [u8] = Box::leak(bytes.into_boxed_slice());
        map.insert(key, Asset { bytes, mime, cache_control: cache });
    }
}

static CACHE: OnceLock<AssetCache> = OnceLock::new();

fn get_cache(static_dir: &str) -> &'static AssetCache {
    let cache = CACHE.get_or_init(|| AssetCache::load(static_dir));
    #[cfg(debug_assertions)]
    debug_assert_eq!(
        cache.dir, static_dir,
        "scheme::handle called with different static_dir after cache init"
    );
    cache
}

/// Pre-warm the asset cache in the calling thread.
/// Call this from a background thread at startup so the first request is instant.
pub fn warm(static_dir: &str) {
    get_cache(static_dir);
}

// ── Request handler ───────────────────────────────────────────────────────────

/// Serve static files via the `bv://` custom protocol.
/// All files are loaded into memory on first request — subsequent requests
/// are served from memory with zero disk I/O.
pub fn handle(static_dir: &str, request: Request<Vec<u8>>) -> Response<Cow<'static, [u8]>> {
    let cache = get_cache(static_dir);

    let uri  = request.uri().to_owned();
    let path = uri.path().trim_start_matches('/');

    // Try exact path, then SPA fallback to index.html
    let asset = cache.get(path).or_else(|| {
        // Only fall back for extensionless paths (HTML routes), not missing assets
        let has_extension = path.contains('.') && !path.ends_with('/');
        if !has_extension { cache.get("index.html") } else { None }
    });

    match asset {
        Some(a) => {
            let mut rsp = Response::builder()
                .status(StatusCode::OK)
                .header("Content-Type", a.mime)
                .header("Cache-Control", a.cache_control);

            if a.mime.starts_with("text/html") {
                rsp = rsp
                    .header("Cross-Origin-Opener-Policy", "same-origin")
                    .header("Cross-Origin-Embedder-Policy", "credentialless");
            }

            rsp.body(Cow::Borrowed(a.bytes)).unwrap()
        }
        None => Response::builder()
            .status(StatusCode::NOT_FOUND)
            .header("Content-Type", "text/plain")
            .body(Cow::Borrowed(b"404 Not Found" as &[u8]))
            .unwrap(),
    }
}

// ── MIME types ────────────────────────────────────────────────────────────────

fn mime_for(path: &std::path::Path) -> &'static str {
    // If the extension is "gz", look at the extension before it
    // e.g. "index.js.gz" → use "js" to determine MIME
    let ext = match path.extension().and_then(|e| e.to_str()) {
        Some("gz") => path
            .file_stem()
            .map(std::path::Path::new)
            .and_then(|p| p.extension())
            .and_then(|e| e.to_str()),
        other => other,
    };

    match ext {
        Some("html") | Some("htm") => "text/html; charset=utf-8",
        Some("js")   | Some("mjs") => "application/javascript; charset=utf-8",
        Some("cjs")                => "application/javascript; charset=utf-8",
        Some("ts")   | Some("tsx") => "application/javascript; charset=utf-8",
        Some("css")                => "text/css; charset=utf-8",
        Some("json")               => "application/json",
        Some("png")                => "image/png",
        Some("jpg")  | Some("jpeg")=> "image/jpeg",
        Some("gif")                => "image/gif",
        Some("svg")                => "image/svg+xml",
        Some("ico")                => "image/x-icon",
        Some("woff")               => "font/woff",
        Some("woff2")              => "font/woff2",
        Some("ttf")                => "font/ttf",
        Some("otf")                => "font/otf",
        Some("webp")               => "image/webp",
        Some("wasm")               => "application/wasm",
        _                          => "application/octet-stream",
    }
}
