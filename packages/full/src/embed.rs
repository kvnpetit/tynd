//! Embedded-assets loader for the full runtime.
//!
//! Full pack entries (order matters — `bun.version` must precede `bun.zst`):
//! `bun.version`, `bun.zst`, `bundle.js`, `frontend/<path>`, `icon.{png,ico}`,
//! `sidecar/<name>`. See `tynd_host::embed` for the wire format.

use std::fs::{self, File};
use std::io::{BufWriter, Read, Write};
use std::path::PathBuf;

use tynd_host::embed::{exe_stem, finalize_sidecars, prepare_extract_dir, PackReader};

pub(crate) struct EmbeddedAssets {
    /// Path to the cached (or fallback system) Bun binary.
    pub bun_path: String,
    pub bundle_path: String,
    pub frontend_dir: String,
    pub icon_path: Option<String>,
}

pub(crate) fn try_load_embedded() -> Option<EmbeddedAssets> {
    let exe = std::env::current_exe().ok()?;
    let mut pack = PackReader::open()?;
    let paths = prepare_extract_dir()?;
    let app_name = exe_stem(&exe);

    let mut bun_path_opt: Option<PathBuf> = None;
    let mut bundle_path_opt: Option<String> = None;
    let mut frontend_dir_opt: Option<String> = None;
    let mut icon_path_opt: Option<String> = None;
    let mut sidecars_pending: Vec<(String, PathBuf)> = Vec::new();

    while let Some((rel, data_len)) = pack.next_header() {
        match rel.as_str() {
            "bun.version" => {
                if data_len > 256 {
                    tynd_host::tynd_log!(
                        "Embedded bun.version entry is suspiciously large ({data_len} bytes) — skipping"
                    );
                    pack.skip(data_len).ok()?;
                    continue;
                }
                let mut ver_bytes = vec![0u8; data_len as usize];
                pack.reader().read_exact(&mut ver_bytes).ok()?;
                let version = String::from_utf8(ver_bytes)
                    .unwrap_or_default()
                    .trim()
                    .to_string();
                bun_path_opt = Some(bun_cache_path(&app_name, &version));
            },

            "bun.zst" => {
                let Some(cache_path) = bun_path_opt.clone() else {
                    pack.skip(data_len).ok()?;
                    continue;
                };
                extract_bun(&mut pack, data_len, &cache_path);
            },

            "bundle.js" => {
                let dest = paths.bundle_dir.join("bundle.js");
                bundle_path_opt = Some(dest.to_string_lossy().into_owned());
                write_entry(&mut pack, data_len, &dest)?;
            },

            "icon.ico" | "icon.png" => {
                let dest = paths.temp_dir.join(&rel);
                icon_path_opt = Some(dest.to_string_lossy().into_owned());
                write_entry(&mut pack, data_len, &dest)?;
            },

            _ if rel.starts_with("frontend/") => {
                let rest = &rel["frontend/".len()..];
                let dest = paths.frontend_dir.join(rest);
                if let Some(parent) = dest.parent() {
                    let _ = fs::create_dir_all(parent);
                }
                if frontend_dir_opt.is_none() {
                    frontend_dir_opt = Some(paths.frontend_dir.to_string_lossy().into_owned());
                }
                write_entry(&mut pack, data_len, &dest)?;
            },

            _ if rel.starts_with("sidecar/") => {
                let rest = &rel["sidecar/".len()..];
                let dest = paths.temp_dir.join("sidecar").join(rest);
                if let Some(parent) = dest.parent() {
                    let _ = fs::create_dir_all(parent);
                }
                write_entry(&mut pack, data_len, &dest)?;
                sidecars_pending.push((rest.to_string(), dest));
            },

            _ => {
                pack.skip(data_len).ok()?;
            },
        }
    }

    let bun_path = match bun_path_opt {
        Some(ref p) if p.exists() => p.to_string_lossy().into_owned(),
        _ => {
            tynd_host::tynd_log!("Cached runtime not available — falling back to system 'bun'");
            "bun".to_string()
        },
    };

    let bundle_path = bundle_path_opt.unwrap_or_else(|| {
        tynd_host::tynd_log!("Embedded pack is missing bundle.js — rebuild with `tynd build`");
        tynd_host::cleanup::run();
        std::process::exit(1);
    });

    let frontend_dir = frontend_dir_opt.unwrap_or_else(|| {
        tynd_host::tynd_log!("Embedded pack is missing frontend files — rebuild with `tynd build`");
        tynd_host::cleanup::run();
        std::process::exit(1);
    });

    finalize_sidecars(&sidecars_pending);

    Some(EmbeddedAssets {
        bun_path,
        bundle_path,
        frontend_dir,
        icon_path: icon_path_opt,
    })
}

fn write_entry(pack: &mut PackReader, data_len: u64, dest: &std::path::Path) -> Option<()> {
    let out = File::create(dest).ok()?;
    let mut writer = BufWriter::new(out);
    std::io::copy(&mut pack.reader().take(data_len), &mut writer).ok()?;
    writer.flush().ok()?;
    Some(())
}

fn extract_bun(pack: &mut PackReader, data_len: u64, cache_path: &std::path::Path) {
    if cache_path.exists() {
        let _ = pack.skip(data_len);
        return;
    }

    eprintln!("First launch — extracting runtime…");

    if let Some(parent) = cache_path.parent() {
        if let Err(e) = fs::create_dir_all(parent) {
            tynd_host::tynd_log!("Failed to create cache dir: {e}");
            let _ = pack.skip(data_len);
            return;
        }
    }

    let Ok(mut decoder) = zstd::Decoder::new(pack.reader().take(data_len)) else {
        tynd_host::tynd_log!("Failed to init zstd decoder");
        return;
    };
    match File::create(cache_path) {
        Ok(out_file) => {
            let mut writer = BufWriter::new(out_file);
            if let Err(e) = std::io::copy(&mut decoder, &mut writer) {
                tynd_host::tynd_log!("Failed to decompress runtime: {e}");
                let _ = fs::remove_file(cache_path);
                std::io::copy(&mut decoder, &mut std::io::sink()).ok();
            } else {
                let _ = writer.flush();
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let _ = fs::set_permissions(cache_path, fs::Permissions::from_mode(0o755));
                }
                eprintln!("Runtime ready.");
            }
        },
        Err(e) => {
            tynd_host::tynd_log!("Failed to create cache file: {e}");
            std::io::copy(&mut decoder, &mut std::io::sink()).ok();
        },
    }
}

/// Returns the persistent cache path for the Bun binary.
///
/// - Windows: `%LOCALAPPDATA%\tynd\{app}\bun-{ver}\bun.exe`
/// - macOS:   `~/Library/Caches/tynd/{app}/bun-{ver}/bun`
/// - Linux:   `~/.cache/tynd/{app}/bun-{ver}/bun`
fn bun_cache_path(app_name: &str, version: &str) -> PathBuf {
    let base = dirs::cache_dir().unwrap_or_else(std::env::temp_dir);
    let bin_name = if cfg!(windows) {
        format!("{app_name}.exe")
    } else {
        app_name.to_string()
    };
    base.join("tynd")
        .join(app_name)
        .join(format!("bun-{version}"))
        .join(bin_name)
}
