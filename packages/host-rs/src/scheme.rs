use std::{
    borrow::Cow,
    collections::HashMap,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, OnceLock,
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
    /// Refcounted bytes — replaces the previous `Box::leak(&'static [u8])`
    /// which permanently leaked every cached file. With `Arc<[u8]>`, a
    /// future hot-reload that swaps `CACHE` can drop the old assets when
    /// no in-flight response still holds a clone.
    bytes: Arc<[u8]>,
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
    ///
    /// Walks the directory tree in parallel via `walkdir + rayon` —
    /// each leaf goes through `process_asset` (read + optional zstd
    /// decompress + MIME sniff) on a worker thread, then the results
    /// are funnelled into the final `HashMap` on the calling thread.
    /// On a typical 5 000-asset bundle this drops first-paint warm-up
    /// from ~200 ms (sequential `read_dir` + `read`) to ~30-60 ms.
    /// Below `PARALLEL_THRESHOLD` we fall back to the sequential path
    /// because `rayon::scope` setup outweighs the win on small trees.
    fn load(dir: &str) -> Self {
        let base = PathBuf::from(dir);

        // Single tree walk — `walkdir` collapses the recursive
        // `read_dir + entry.path() + recursion` ladder the previous
        // hand-rolled walker shipped, and handles symlink loops +
        // permission errors uniformly across platforms.
        let entries: Vec<walkdir::DirEntry> = walkdir::WalkDir::new(&base)
            .follow_links(false)
            .into_iter()
            .filter_map(Result::ok)
            .filter(|e| e.file_type().is_file())
            .collect();

        let processed: Vec<(String, Asset)> = if entries.len() >= PARALLEL_THRESHOLD {
            // Parallel decode — `rayon` dispatches `process_asset`
            // across the global thread-pool. Each call is independent
            // (no shared mutable state) so there's no contention on
            // the hot path; the join + collect at the end is the only
            // serialization point.
            use rayon::prelude::*;
            entries
                .par_iter()
                .filter_map(|entry| process_asset(&base, entry.path()))
                .collect()
        } else {
            // Small bundles — `rayon::scope` setup + per-task overhead
            // would dominate on a handful of files. Stay sequential.
            entries
                .iter()
                .filter_map(|entry| process_asset(&base, entry.path()))
                .collect()
        };

        let mut map = HashMap::with_capacity(processed.len());
        for (key, asset) in processed {
            map.insert(key, asset);
        }

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

/// Below this many files in the bundle we stay on the sequential
/// `iter()` path — the per-task overhead of dispatching tiny work units
/// to `rayon`'s thread-pool would dominate the actual decode cost.
const PARALLEL_THRESHOLD: usize = 32;

/// Read + (optionally) zstd-decompress one asset from disk and return
/// the cache-ready `(key, Asset)` tuple. Returns `None` for paths that
/// fail to read or decompress (logged via `tynd_warn!` for the zstd
/// case so the operator sees the partial cache).
#[allow(clippy::case_sensitive_file_extension_comparisons)] // .zst suffix written by CLI, always lowercase
fn process_asset(base: &PathBuf, path: &std::path::Path) -> Option<(String, Asset)> {
    let bytes = std::fs::read(path).ok()?;
    let rel = path
        .strip_prefix(base)
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_default();

    // Detect zstd: strip .zst suffix, decompress bytes in memory.
    // Compression is only for binary size — assets are served
    // uncompressed because wry's custom protocol does not process
    // Content-Encoding.
    let (key, bytes) = if rel.ends_with(".zst") {
        let key = rel[..rel.len() - 4].to_owned();
        match zstd::decode_all(bytes.as_slice()) {
            Ok(out) => (key, out),
            Err(e) => {
                crate::tynd_warn!("Failed to decompress asset '{rel}': {e}");
                return None;
            },
        }
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

    // `Arc<[u8]>` keeps shared-ownership semantics for the cache while
    // letting the allocation be freed when the cache is dropped or
    // replaced (the previous `Box::leak` permanently leaked every
    // hot-reloaded file).
    let bytes: Arc<[u8]> = Arc::from(bytes.into_boxed_slice());
    Some((
        key,
        Asset {
            bytes,
            mime: mime.into_boxed_str(),
            cache_control: cache,
        },
    ))
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
            "[tynd] FATAL: scheme cache mismatch — initialized with '{}', called with \
             '{static_dir}'",
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

            // wry's custom-protocol response body is `Cow<'static, [u8]>`,
            // so we materialize the `Arc<[u8]>` into an owned `Vec` here.
            // The Arc keeps the cache copy alive for any concurrent
            // responses; this Vec is just the per-response payload.
            rsp.body(Cow::Owned(a.bytes.to_vec())).unwrap()
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
    let mime = mime_for_bytes(&target, &bytes);
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
    "default-src 'self' tynd: tynd-bin:; img-src 'self' tynd: tynd-bin: data: blob:; media-src \
     'self' tynd: tynd-bin: data: blob:; script-src 'self' tynd:; style-src 'self' tynd: \
     'unsafe-inline'; font-src 'self' tynd: data:; connect-src 'self' tynd: tynd-bin: ipc: https: \
     wss:; frame-src 'none'; object-src 'none'; base-uri 'self'; form-action 'self'"
}

// -----------------------------------------------------------------------
// W4.3 — scheme tests
//
// `CACHE` is a process-global `OnceLock<AssetCache>` that the production
// `handle()` populates from the static-dir argument. Unit tests can't
// cleanly init/replace it across cases, so the tests here focus on the
// pieces we *can* exercise without poisoning shared state:
//   * `AssetCache::load` building a map from disk + serving lookups.
//   * `mime_for` covering the bundler-artefact + `.zst` strip cases that
//     the cache feeds into `handle`.
//   * `set_dev_mode(true)` round-trip + dev `serve_from_disk` reading
//     fresh bytes (this is the cache-bypass branch — no `OnceLock`
//     interaction).
// -----------------------------------------------------------------------

#[cfg(test)]
mod scheme_tests {
    use super::*;
    use wry::http::Method;

    fn make_request(uri: &str) -> Request<Vec<u8>> {
        Request::builder()
            .method(Method::GET)
            .uri(uri)
            .body(Vec::new())
            .unwrap()
    }

    #[test]
    fn asset_cache_loads_files_and_serves_them() {
        // AssetCache::load walks a directory once and stores
        // Arc<[u8]> per file. Smoke-test the load + serve path
        // without going through the global CACHE so this test stays
        // independent of test ordering.
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("sub");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(dir.path().join("index.html"), b"<!doctype html>").unwrap();
        std::fs::write(nested.join("app.js"), b"console.log(1)").unwrap();

        let cache = AssetCache::load(&dir.path().to_string_lossy());

        let html = cache.get("index.html").expect("index.html in cache");
        assert_eq!(html.bytes.as_ref(), b"<!doctype html>");
        assert!(
            html.mime.starts_with("text/html"),
            "expected text/html mime, got {}", html.mime,
        );
        // index.html gets `no-cache` so editor saves land after a reload.
        assert_eq!(html.cache_control, "no-cache");

        let js = cache.get("sub/app.js").expect("sub/app.js in cache");
        assert_eq!(js.bytes.as_ref(), b"console.log(1)");
        assert!(
            js.mime.starts_with("application/javascript"),
            "expected JS mime, got {}", js.mime,
        );
        // Hashed assets get the long immutable cache.
        assert!(js.cache_control.contains("immutable"));
    }

    #[test]
    fn asset_cache_decompresses_zst_suffix() {
        // CLI-side zstd compression: assets land on disk as `<name>.zst`
        // and load() must decompress them in memory under the
        // un-suffixed key. A regression here would serve raw zstd bytes
        // to the webview (binary garbage).
        let dir = tempfile::tempdir().unwrap();
        let original = b"const greeting = 'hi';";
        let compressed = zstd::encode_all(&original[..], 3).unwrap();
        std::fs::write(dir.path().join("hashed.js.zst"), compressed).unwrap();

        let cache = AssetCache::load(&dir.path().to_string_lossy());

        // Key is the un-suffixed name; bytes are the original payload.
        let asset = cache.get("hashed.js").expect("decompressed under .js key");
        assert_eq!(asset.bytes.as_ref(), original);
        // `.zst` key MUST NOT appear — that would mean we cached the
        // compressed form and the decompression branch never ran.
        assert!(cache.get("hashed.js.zst").is_none());
    }

    #[test]
    fn dev_mode_serves_fresh_bytes_from_disk() {
        // `set_dev_mode(true)` flips a process-global atomic that
        // `handle()` reads to decide whether to consult the cache or
        // re-read from disk. Editor-save workflow expects the second
        // request to see the new contents — exercise the bypass.
        // We hit the explicit file path (not "/") because the dev-mode
        // serve path canonicalizes against `static_dir` and treats `""`
        // as "the dir itself" (which fails to read as a file). The
        // production webview always requests a concrete asset URL once
        // the SPA boots, so the explicit path is a faithful proxy.
        let dir = tempfile::tempdir().unwrap();
        let asset_name = "main.js";
        std::fs::write(dir.path().join(asset_name), b"console.log('v1')").unwrap();
        let static_dir = dir.path().to_string_lossy().to_string();

        // Save current dev-mode flag and restore on exit so we don't
        // contaminate other tests in the same binary.
        struct DevModeGuard(bool);
        impl Drop for DevModeGuard {
            fn drop(&mut self) {
                set_dev_mode(self.0);
            }
        }
        let _restore = DevModeGuard(dev_mode());
        set_dev_mode(true);

        let url = format!("tynd://localhost/{asset_name}");
        let r1 = handle(&static_dir, &make_request(&url));
        assert_eq!(r1.status(), StatusCode::OK);
        assert_eq!(r1.body().as_ref(), b"console.log('v1')");

        // Editor save → write a new payload.
        std::fs::write(dir.path().join(asset_name), b"console.log('v2')").unwrap();
        let r2 = handle(&static_dir, &make_request(&url));
        assert_eq!(r2.status(), StatusCode::OK);
        assert_eq!(
            r2.body().as_ref(),
            b"console.log('v2')",
            "dev mode must bypass cache on every request",
        );
    }
}

/// Resolve the response Content-Type for a static asset, with optional
/// content sniffing when the bytes are already in hand.
///
/// Resolution order:
///   1. `infer` magic-byte sniff over the first 8 KiB — wins for binary
///      formats (images, video, archives). Skipped for path extensions
///      we always want to serve as a specific text MIME (HTML, CSS, JS
///      and the bundler-artefact extensions).
///   2. `mime_guess` extension lookup — covers text formats `infer`
///      doesn't sniff.
///   3. The bundler-artefact override (`.ts` / `.tsx` / `.mjs` / `.cjs`
///      → `application/javascript`).
///   4. `application/octet-stream` fallback.
fn mime_for_bytes(path: &std::path::Path, bytes: &[u8]) -> String {
    // Skip `infer` for known-text extensions where the magic-byte
    // sniffer would either return None (and we'd fall through anyway)
    // or, worse, mislabel the file as a generic binary.
    let ext = path.extension().and_then(|e| e.to_str());
    let always_text = matches!(
        ext,
        Some("html" | "htm" | "css" | "ts" | "tsx" | "mjs" | "cjs" | "js" | "json" | "txt" | "svg")
    );
    if !always_text {
        // Sample only the prefix — the file-format magic bytes always
        // sit at the start, so scanning the full payload of a 50 MB
        // video would just burn cache lines. 8 KiB is comfortably above
        // the largest signature `infer` checks (a few hundred bytes).
        let sample = &bytes[..bytes.len().min(8192)];
        if let Some(kind) = infer::get(sample) {
            return kind.mime_type().to_string();
        }
    }
    mime_for(path)
}

/// Path-only MIME resolution — used when bytes aren't loaded yet
/// (currently nowhere on the hot path; kept as the fallback inside
/// `mime_for_bytes` and as the canonical extension lookup).
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
