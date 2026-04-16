//! Embedded-assets support for `vorn build` (full runtime).
//!
//! `vorn build` appends a packed section to the vorn-full binary:
//!
//! ```text
//! [vorn-full binary]
//! ┌─ packed section ──────────────────────────────────────────────────┐
//! │  file_count : u32 LE                                              │
//! │  for each file:                                                   │
//! │    path_len : u16 LE                                              │
//! │    path     : UTF-8 bytes                                         │
//! │    data_len : u32 LE                                              │
//! │    data     : raw bytes                                           │
//! └───────────────────────────────────────────────────────────────────┘
//! section_size : u64 LE   (byte count of the packed section above)
//! magic        : "VORNPKG\0" (8 bytes)
//! ```
//!
//! Entry names (ORDER MATTERS — bun.version MUST be first):
//!   - "bun.version"        — UTF-8 version string, used as cache key
//!   - "bun.gz"             — gzip-compressed Bun binary
//!   - "bundle.js"          — backend JS bundle (plain, not gzipped)
//!   - "frontend/<path>.gz" — gzipped frontend assets
//!   - "icon.ico"           — optional application icon
//!
//! At startup, `try_load_embedded()` checks for the magic trailer.
//! If found it extracts assets, caching the Bun binary persistently and
//! writing other files to a temp directory.

use std::fs;
use std::io::{BufWriter, Read, Seek, SeekFrom, Write};
use std::path::PathBuf;

use flate2::read::GzDecoder;

const MAGIC: &[u8; 8] = b"VORNPKG\0";
/// section_size(u64) + magic(8) = 16 bytes
const TRAILER_LEN: u64 = 16;

pub(crate) struct EmbeddedAssets {
    /// Path to the cached (or fallback system) Bun binary.
    pub bun_path: String,
    /// Path to the extracted bundle.js in the temp directory.
    pub bundle_path: String,
    /// Path to the temp frontend directory.
    pub frontend_dir: String,
    /// Optional extracted icon file.
    pub icon_path: Option<String>,
}

/// Try to read embedded assets from the end of the running executable.
/// Returns `None` if no assets are appended (normal dev-mode invocation).
pub(crate) fn try_load_embedded() -> Option<EmbeddedAssets> {
    let exe = std::env::current_exe().ok()?;
    let mut f = fs::File::open(&exe).ok()?;
    let size = f.metadata().ok()?.len();

    if size < TRAILER_LEN {
        return None;
    }

    f.seek(SeekFrom::End(-8)).ok()?;
    let mut magic = [0u8; 8];
    f.read_exact(&mut magic).ok()?;
    if &magic != MAGIC {
        return None;
    }

    f.seek(SeekFrom::End(-16)).ok()?;
    let mut sz = [0u8; 8];
    f.read_exact(&mut sz).ok()?;
    let section_size = u64::from_le_bytes(sz);

    if size < TRAILER_LEN + section_size {
        return None;
    }

    let section_start = size - TRAILER_LEN - section_size;
    f.seek(SeekFrom::Start(section_start)).ok()?;

    let mut cb = [0u8; 4];
    f.read_exact(&mut cb).ok()?;
    let file_count = u32::from_le_bytes(cb) as usize;

    // `std::mem::forget` prevents TempDir from auto-deleting on drop.
    // Cleanup is instead registered with vorn_host::cleanup so it runs
    // before every process::exit (which bypasses Rust's drop machinery).
    let td = tempfile::TempDir::with_prefix("vorn-").ok()?;
    let temp_dir = td.path().to_owned();
    #[allow(clippy::mem_forget)] // intentional: cleanup::run() handles removal across exit paths
    std::mem::forget(td);
    vorn_host::cleanup::register_dir(temp_dir.clone());

    let bundle_dir = temp_dir.join("backend");
    let frontend_dir = temp_dir.join("frontend");
    fs::create_dir_all(&bundle_dir).ok()?;
    fs::create_dir_all(&frontend_dir).ok()?;

    let app_name = exe
        .file_stem()
        .map_or_else(|| "app".to_string(), |s| s.to_string_lossy().into_owned());

    let mut bun_path_opt: Option<PathBuf> = None;
    let mut bundle_path_opt: Option<String> = None;
    let mut frontend_dir_opt: Option<String> = None;
    let mut icon_path_opt: Option<String> = None;

    for _ in 0..file_count {
        // Read path
        let mut pl = [0u8; 2];
        f.read_exact(&mut pl).ok()?;
        let path_len = u16::from_le_bytes(pl) as usize;
        let mut path_buf = vec![0u8; path_len];
        f.read_exact(&mut path_buf).ok()?;
        let rel = String::from_utf8(path_buf).ok()?;

        // Read data length
        let mut dl = [0u8; 4];
        f.read_exact(&mut dl).ok()?;
        let data_len = u32::from_le_bytes(dl) as u64;

        match rel.as_str() {
            "bun.version" => {
                // Sanity-check: a valid version string is never longer than 256 bytes.
                // Reject oversized entries to prevent allocating a giant buffer on
                // a corrupted or tampered binary.
                if data_len > 256 {
                    vorn_host::vorn_log!("Embedded bun.version entry is suspiciously large ({data_len} bytes) — skipping");
                    let mut skip = (&mut f).take(data_len);
                    std::io::copy(&mut skip, &mut std::io::sink()).ok()?;
                    continue;
                }
                let mut ver_bytes = vec![0u8; data_len as usize];
                f.read_exact(&mut ver_bytes).ok()?;
                let version = String::from_utf8(ver_bytes).unwrap_or_default();
                let version = version.trim().to_string();
                let cache_path = bun_cache_path(&app_name, &version);
                bun_path_opt = Some(cache_path);
            },

            "bun.gz" => {
                let cache_path = if let Some(p) = &bun_path_opt {
                    p.clone()
                } else {
                    // bun.version was missing — fallback: skip and use system bun
                    let mut skip = (&mut f).take(data_len);
                    std::io::copy(&mut skip, &mut std::io::sink()).ok()?;
                    continue;
                };

                if cache_path.exists() {
                    // Cache hit — skip bytes without reading them into memory
                    let mut skip = (&mut f).take(data_len);
                    std::io::copy(&mut skip, &mut std::io::sink()).ok()?;
                } else {
                    // Cache miss — decompress and write
                    eprintln!("First launch — extracting runtime…");

                    if let Some(parent) = cache_path.parent() {
                        if let Err(e) = fs::create_dir_all(parent) {
                            vorn_host::vorn_log!("Failed to create cache dir: {e}");
                            let mut skip = (&mut f).take(data_len);
                            std::io::copy(&mut skip, &mut std::io::sink()).ok()?;
                            continue;
                        }
                    }

                    let compressed_reader = (&mut f).take(data_len);
                    let mut decoder = GzDecoder::new(compressed_reader);
                    match fs::File::create(&cache_path) {
                        Ok(out_file) => {
                            let mut writer = BufWriter::new(out_file);
                            if let Err(e) = std::io::copy(&mut decoder, &mut writer) {
                                vorn_host::vorn_log!("Failed to decompress runtime: {e}");
                                let _ = fs::remove_file(&cache_path);
                                // Drain remaining compressed bytes so the file cursor
                                // lands at the next pack entry, not mid-block.
                                std::io::copy(&mut decoder, &mut std::io::sink()).ok();
                            } else {
                                let _ = writer.flush();
                                // Make executable on Unix
                                #[cfg(unix)]
                                {
                                    use std::os::unix::fs::PermissionsExt;
                                    let _ = fs::set_permissions(
                                        &cache_path,
                                        fs::Permissions::from_mode(0o755),
                                    );
                                }
                                eprintln!("Runtime ready.");
                            }
                        },
                        Err(e) => {
                            vorn_host::vorn_log!("Failed to create cache file: {e}");
                            // Drain remaining compressed bytes
                            let mut drain = decoder;
                            std::io::copy(&mut drain, &mut std::io::sink()).ok();
                        },
                    }
                }
            },

            "bundle.js" => {
                let dest = bundle_dir.join("bundle.js");
                bundle_path_opt = Some(dest.to_string_lossy().into_owned());
                let out = fs::File::create(&dest).ok()?;
                let mut w = BufWriter::new(out);
                std::io::copy(&mut (&mut f).take(data_len), &mut w).ok()?;
                w.flush().ok()?;
            },

            _ if rel.starts_with("frontend/") => {
                let rest = &rel["frontend/".len()..];
                let dest = frontend_dir.join(rest);
                if let Some(parent) = dest.parent() {
                    let _ = fs::create_dir_all(parent);
                }
                if frontend_dir_opt.is_none() {
                    frontend_dir_opt = Some(frontend_dir.to_string_lossy().into_owned());
                }
                let out = fs::File::create(&dest).ok()?;
                let mut w = BufWriter::new(out);
                std::io::copy(&mut (&mut f).take(data_len), &mut w).ok()?;
                w.flush().ok()?;
            },

            "icon.ico" | "icon.png" => {
                let dest = temp_dir.join(&rel);
                icon_path_opt = Some(dest.to_string_lossy().into_owned());
                let out = fs::File::create(&dest).ok()?;
                let mut w = BufWriter::new(out);
                std::io::copy(&mut (&mut f).take(data_len), &mut w).ok()?;
                w.flush().ok()?;
            },

            _ => {
                let mut skip = (&mut f).take(data_len);
                std::io::copy(&mut skip, &mut std::io::sink()).ok()?;
            },
        }
    }

    // Resolve Bun path — use cache path if the file now exists, else fall back
    let bun_path = match bun_path_opt {
        Some(ref p) if p.exists() => p.to_string_lossy().into_owned(),
        _ => {
            vorn_host::vorn_log!("Cached runtime not available — falling back to system 'bun'");
            "bun".to_string()
        },
    };

    let bundle_path = bundle_path_opt.unwrap_or_else(|| {
        vorn_host::vorn_log!("Embedded pack is missing bundle.js — rebuild with `vorn build`");
        vorn_host::cleanup::run();
        std::process::exit(1);
    });

    let frontend_dir = frontend_dir_opt.unwrap_or_else(|| {
        vorn_host::vorn_log!("Embedded pack is missing frontend files — rebuild with `vorn build`");
        vorn_host::cleanup::run();
        std::process::exit(1);
    });

    Some(EmbeddedAssets {
        bun_path,
        bundle_path,
        frontend_dir,
        icon_path: icon_path_opt,
    })
}

/// Returns the persistent cache path for the Bun binary.
///
/// - Windows: `%LOCALAPPDATA%\vorn\{app}\bun-{ver}\bun.exe`
/// - macOS:   `~/Library/Caches/vorn/{app}/bun-{ver}/bun`
/// - Linux:   `~/.cache/vorn/{app}/bun-{ver}/bun`
fn bun_cache_path(app_name: &str, version: &str) -> PathBuf {
    let base = dirs::cache_dir().unwrap_or_else(std::env::temp_dir);
    let bin_name = if cfg!(windows) {
        format!("{app_name}.exe")
    } else {
        app_name.to_string()
    };
    base.join("vorn")
        .join(app_name)
        .join(format!("bun-{version}"))
        .join(bin_name)
}
