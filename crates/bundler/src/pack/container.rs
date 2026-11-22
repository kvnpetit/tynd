use std::{
    collections::HashSet,
    fs::File,
    io::{BufWriter, Write},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use error_stack::{Report, ResultExt as _};
use rayon::prelude::*;
use tracing::info;

use super::{
    icons::MAX_ICON_BYTES, read_file_limited, PackConfig, PackContext, PackError, SidecarSpec,
};

const TEXT_EXTS: &[&str] = &["html", "htm", "js", "mjs", "cjs", "css", "json", "svg"];
/// Level 19 saves 1-3 MB/build versus level 3 for a one-time CI cost.
const ZSTD_LEVEL: i32 = 19;
const MAX_BUN_RUNTIME_BYTES: u64 = 1024 * 1024 * 1024;
const MAX_PACK_ENTRY_BYTES: u64 = 512 * 1024 * 1024;
const MAX_PACK_TOTAL_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const MAX_BACKEND_BUNDLE_BYTES: u64 = 128 * 1024 * 1024;
const MAX_FRONTEND_ENTRY_BYTES: u64 = 128 * 1024 * 1024;
// Entry collection

/// `Raw` carries bytes; `Disk` defers read+compress until the packing pass
/// so rayon can parallelise the slow zstd-encode path.
pub(super) enum PackEntry {
    Raw { rel: String, data: Vec<u8> },
    Disk { rel: String, abs: PathBuf },
}

impl PackEntry {
    pub(super) fn raw(rel: impl Into<String>, data: Vec<u8>) -> Self {
        Self::Raw {
            rel: rel.into(),
            data,
        }
    }
    fn disk(rel: impl Into<String>, abs: PathBuf) -> Self {
        Self::Disk {
            rel: rel.into(),
            abs,
        }
    }
}

pub(super) fn collect_entries(cfg: &PackConfig) -> Result<Vec<PackEntry>> {
    let mut entries = Vec::new();

    append_bun_entries(cfg, &mut entries)?;
    append_webview2_entry(cfg, &mut entries)?;
    append_observability_entry(cfg, &mut entries)?;
    if let Some(flag) = tynd_pack_format::debug_flag_entry(cfg.debug) {
        entries.push(PackEntry::raw(flag.rel, flag.data.to_vec()));
    }

    // Backend bundle stays raw; Bun starts it directly from extracted assets.
    let backend = read_file_limited(&cfg.backend_bundle, MAX_BACKEND_BUNDLE_BYTES)?;
    entries.push(PackEntry::raw("bundle.js", backend));

    append_frontend_entries(&cfg.frontend_dir, &mut entries)?;
    append_sidecar_entries(&cfg.sidecars, &mut entries)?;
    Ok(entries)
}

fn append_bun_entries(cfg: &PackConfig, entries: &mut Vec<PackEntry>) -> Result<()> {
    let Some(bun) = &cfg.bun_runtime else {
        return Ok(());
    };

    // Bun metadata must precede `bun.zst` — the host validates both before
    // accepting or extracting the cached runtime.
    let version = cfg
        .bun_version
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("bun_version is required when bun_runtime is set"))?;
    validate_bun_version(version)?;
    entries.push(PackEntry::raw("bun.version", version.as_bytes().to_vec()));

    let bun_disp = bun.display().to_string();
    let bun_bytes = read_file_limited(bun, MAX_BUN_RUNTIME_BYTES)?;
    let raw_len = bun_bytes.len();
    let bun_hash = blake3::hash(&bun_bytes);
    entries.push(PackEntry::raw(
        "bun.blake3",
        bun_hash.to_hex().to_string().into_bytes(),
    ));
    let cache_entry = super::bun_cache::default_dir()
        .map(|dir| super::bun_cache::entry_path(&dir, &bun_hash, raw_len as u64, ZSTD_LEVEL));
    if let Some(zst) = cache_entry
        .as_deref()
        .and_then(|path| super::bun_cache::load(path, &bun_hash, raw_len as u64))
    {
        info!(
            "bun runtime: reusing compressed cache ({:.0} MB)",
            zst.len() as f64 / 1_048_576.0
        );
        entries.push(PackEntry::raw("bun.zst", zst));
        return Ok(());
    }

    info!("compressing bun runtime…");
    let zst = zstd::encode_all(bun_bytes.as_slice(), ZSTD_LEVEL)
        .map_err(Report::from)
        .change_context(PackContext::CompressBunRuntime)
        .attach_with({
            let path = bun_disp.clone();
            move || format!("bun_path = {path}")
        })
        .attach_with(move || format!("raw_size = {raw_len}"))
        .attach_with(move || format!("zstd_level = {ZSTD_LEVEL}"))
        .map_err(|report| {
            anyhow::Error::new(PackError::Io {
                path: bun_disp.clone(),
                cause: format!("zstd encode: {report:?}"),
            })
        })?;
    info!(
        "bun runtime compressed: {:.0} MB",
        zst.len() as f64 / 1_048_576.0
    );
    if let Some(path) = &cache_entry {
        super::bun_cache::store(path, &zst);
    }
    entries.push(PackEntry::raw("bun.zst", zst));
    Ok(())
}

fn validate_bun_version(version: &str) -> Result<()> {
    if version.is_empty()
        || version.len() > 64
        || version == "."
        || version == ".."
        || !version
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b".-+_".contains(&byte))
    {
        anyhow::bail!("bun_version must be a safe 1..=64 byte version identifier, got {version:?}");
    }
    Ok(())
}

fn append_webview2_entry(cfg: &PackConfig, entries: &mut Vec<PackEntry>) -> Result<()> {
    let Some(path) = &cfg.webview2_runtime_path else {
        return Ok(());
    };
    if path.len() > tynd_pack_format::MAX_BOOT_METADATA_BYTES as usize
        || tynd_pack_format::validate_subpath(path).is_none()
    {
        return Err(anyhow::Error::new(PackError::InvalidAssetPath {
            path: format!("webview2 runtime: {path}"),
        }));
    }
    entries.push(PackEntry::raw(
        tynd_pack_format::WEBVIEW2_RUNTIME_PATH_ENTRY,
        path.as_bytes().to_vec(),
    ));
    Ok(())
}

/// Entry name read by `tynd_runtime` (`crate::observability::PACK_ENTRY`).
const OBSERVABILITY_ENTRY: &str = "observability.json";

fn append_observability_entry(cfg: &PackConfig, entries: &mut Vec<PackEntry>) -> Result<()> {
    let Some(json) = &cfg.observability_json else {
        return Ok(());
    };
    if json.len() > tynd_pack_format::MAX_BOOT_METADATA_BYTES as usize {
        anyhow::bail!(
            "observability config is {} bytes; the {OBSERVABILITY_ENTRY} entry is capped at {}",
            json.len(),
            tynd_pack_format::MAX_BOOT_METADATA_BYTES
        );
    }
    if !serde_json::from_str::<serde_json::Value>(json).is_ok_and(|value| value.is_object()) {
        anyhow::bail!("observability config must be a JSON object");
    }
    entries.push(PackEntry::raw(
        OBSERVABILITY_ENTRY,
        json.as_bytes().to_vec(),
    ));
    Ok(())
}

fn append_frontend_entries(root: &Path, entries: &mut Vec<PackEntry>) -> Result<()> {
    // Forward-slash paths — host reader expects POSIX regardless of build OS.
    for (rel, abs) in collect_frontend_files(root)? {
        entries.push(PackEntry::disk(format!("frontend/{rel}"), abs));
    }
    Ok(())
}

fn append_sidecar_entries(sidecars: &[SidecarSpec], entries: &mut Vec<PackEntry>) -> Result<()> {
    let mut seen_names: HashSet<&str> = HashSet::new();
    for sidecar in sidecars {
        if !seen_names.insert(sidecar.name.as_str()) {
            return Err(anyhow::Error::new(PackError::DuplicateAssetPath {
                path: format!("sidecar/{}", sidecar.name),
            }));
        }
        if tynd_pack_format::validate_subpath(&sidecar.name).is_none() {
            return Err(anyhow::Error::new(PackError::InvalidAssetPath {
                path: format!("sidecar/{}", sidecar.name),
            }));
        }
        entries.push(PackEntry::disk(
            format!("sidecar/{}", sidecar.name),
            sidecar.path.clone(),
        ));
    }
    Ok(())
}
fn collect_frontend_files(root: &Path) -> Result<Vec<(String, PathBuf)>> {
    use walkdir::WalkDir;
    let mut out = Vec::new();
    for entry in WalkDir::new(root).sort_by_file_name().follow_links(false) {
        let entry = entry.map_err(|error| {
            anyhow::Error::new(PackError::Io {
                path: root.display().to_string(),
                cause: error.to_string(),
            })
        })?;
        if entry.file_type().is_symlink() {
            return Err(anyhow::Error::new(PackError::Io {
                path: entry.path().display().to_string(),
                cause: "symbolic links are not allowed in frontend assets".into(),
            }));
        }
        if !entry.file_type().is_file() {
            continue;
        }
        let abs = entry.path().to_path_buf();
        let rel = match abs.strip_prefix(root) {
            Ok(r) => r.to_path_buf(),
            Err(error) => {
                return Err(anyhow::Error::new(PackError::Io {
                    path: abs.display().to_string(),
                    cause: error.to_string(),
                }));
            },
        };
        let rel_str = rel
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("/");
        out.push((rel_str, abs));
    }
    Ok(out)
}

// TYNDPKG section assembly

pub(super) fn write_packed(out: &Path, host: &[u8], entries: &[PackEntry]) -> Result<()> {
    // Read+zstd-19 dominates wall time; rayon preserves order.
    let prepared: Vec<(String, Vec<u8>)> = entries
        .par_iter()
        .map(prepare_entry)
        .collect::<Result<Vec<_>>>()?;

    validate_prepared_paths(&prepared)?;
    validate_prepared_sizes(&prepared)?;

    let out_disp = out.display().to_string();
    let format_entries: Vec<_> = prepared
        .iter()
        .map(|(rel, data)| tynd_pack_format::PackEntry::new(rel.as_str(), data.as_slice()))
        .collect();
    tynd_pack_format::validate_entries(&format_entries)
        .map_err(|error| map_pack_write_error(error, &out_disp))?;

    let mut writer = create_packed_writer(out, &out_disp, host.len(), prepared.len())?;
    writer.write_all(host).context("write host")?;
    tynd_pack_format::write_section_and_trailer(&mut writer, format_entries)
        .map_err(|error| map_pack_write_error(error, &out_disp))?;
    writer.flush().context("flush packed file")?;
    Ok(())
}

fn validate_prepared_paths(prepared: &[(String, Vec<u8>)]) -> Result<()> {
    let mut seen_paths = HashSet::with_capacity(prepared.len());
    for (rel, _) in prepared {
        if tynd_pack_format::validate_subpath(rel).is_none() {
            return Err(anyhow::Error::new(PackError::InvalidAssetPath {
                path: rel.clone(),
            }));
        }
        if !seen_paths.insert(rel.as_str()) {
            return Err(anyhow::Error::new(PackError::DuplicateAssetPath {
                path: rel.clone(),
            }));
        }
    }
    Ok(())
}

fn validate_prepared_sizes(prepared: &[(String, Vec<u8>)]) -> Result<()> {
    let mut total_bytes = 0_u64;
    for (rel, data) in prepared {
        let len = data.len() as u64;
        if len > entry_size_limit(rel) {
            return Err(anyhow::Error::new(PackError::AssetTooLarge {
                len,
                path: rel.clone(),
            }));
        }
        total_bytes = total_bytes.checked_add(len).ok_or_else(|| {
            anyhow::Error::new(PackError::AssetTooLarge {
                len: u64::MAX,
                path: "<pack total>".into(),
            })
        })?;
        if total_bytes > MAX_PACK_TOTAL_BYTES {
            return Err(anyhow::Error::new(PackError::AssetTooLarge {
                len: total_bytes,
                path: "<pack total>".into(),
            }));
        }
    }
    Ok(())
}

fn entry_size_limit(rel: &str) -> u64 {
    if rel == "bundle.js" {
        MAX_BACKEND_BUNDLE_BYTES
    } else if rel == "icon.ico" || rel == "icon.png" {
        MAX_ICON_BYTES
    } else if rel.starts_with("frontend/") {
        MAX_FRONTEND_ENTRY_BYTES
    } else {
        MAX_PACK_ENTRY_BYTES
    }
}

fn create_packed_writer(
    out: &Path,
    out_disp: &str,
    host_size: usize,
    entry_count: usize,
) -> Result<BufWriter<File>> {
    let file = File::create(out)
        .map_err(Report::from)
        .change_context(PackContext::WritePackedFile)
        .attach_with({
            let path = out_disp.to_owned();
            move || format!("output_path = {path}")
        })
        .attach_with(move || format!("host_size = {host_size}"))
        .attach_with(move || format!("entry_count = {entry_count}"))
        .map_err(|report| {
            anyhow::Error::new(PackError::Io {
                path: out_disp.to_owned(),
                cause: format!("{report:?}"),
            })
        })?;
    Ok(BufWriter::with_capacity(1 << 20, file))
}
fn map_pack_write_error(err: tynd_pack_format::PackWriteError, out: &str) -> anyhow::Error {
    match err {
        tynd_pack_format::PackWriteError::PathTooLong { len, path } => {
            anyhow::Error::new(PackError::PathTooLong { len, path })
        },
        tynd_pack_format::PackWriteError::DataTooLarge { len, path } => {
            anyhow::Error::new(PackError::AssetTooLarge { len, path })
        },
        other => anyhow::Error::new(PackError::Io {
            path: out.to_owned(),
            cause: other.to_string(),
        }),
    }
}

fn prepare_entry(entry: &PackEntry) -> Result<(String, Vec<u8>)> {
    match entry {
        PackEntry::Raw { rel, data } => Ok((rel.clone(), data.clone())),
        PackEntry::Disk { rel, abs } => {
            let raw_cap = if rel.starts_with("frontend/") {
                MAX_FRONTEND_ENTRY_BYTES
            } else {
                MAX_PACK_ENTRY_BYTES
            };
            let raw = read_file_limited(abs, raw_cap)?;
            // `.zst` suffix tells `crates/host/src/api/http` to stream-decode.
            let ext = abs
                .extension()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            if TEXT_EXTS.iter().any(|e| *e == ext) {
                let zst = zstd::encode_all(raw.as_slice(), ZSTD_LEVEL).map_err(|e| {
                    anyhow::Error::new(PackError::Io {
                        path: abs.display().to_string(),
                        cause: format!("zstd encode: {e}"),
                    })
                })?;
                Ok((format!("{rel}.zst"), zst))
            } else {
                Ok((rel.clone(), raw))
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn config(observability_json: Option<&str>) -> PackConfig {
        serde_json::from_value(json!({
            "host_binary": "host",
            "output": "out",
            "app_name": "demo",
            "version": "1.0.0",
            "backend_bundle": "bundle.js",
            "frontend_dir": "frontend",
            "observability_json": observability_json,
        }))
        .expect("pack config deserializes")
    }

    fn raw_entries(entries: &[PackEntry]) -> Vec<(&str, &[u8])> {
        entries
            .iter()
            .filter_map(|entry| match entry {
                PackEntry::Raw { rel, data } => Some((rel.as_str(), data.as_slice())),
                PackEntry::Disk { .. } => None,
            })
            .collect()
    }

    #[test]
    fn no_observability_adds_no_entry() {
        let mut entries = Vec::new();
        append_observability_entry(&config(None), &mut entries).unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn observability_is_packed_verbatim() {
        let json = r#"{"sentry":{"dsn":"https://k@o.example.com/1"}}"#;
        let mut entries = Vec::new();
        append_observability_entry(&config(Some(json)), &mut entries).unwrap();
        assert_eq!(
            raw_entries(&entries),
            vec![("observability.json", json.as_bytes())]
        );
    }

    #[test]
    fn oversized_or_malformed_observability_is_rejected() {
        let mut entries = Vec::new();
        let huge = format!(r#"{{"sentry":{{"dsn":"{}"}}}}"#, "x".repeat(5000));
        assert!(append_observability_entry(&config(Some(&huge)), &mut entries).is_err());
        assert!(append_observability_entry(&config(Some("[]")), &mut entries).is_err());
        assert!(append_observability_entry(&config(Some("{broken")), &mut entries).is_err());
        assert!(entries.is_empty());
    }

    #[test]
    fn debug_flag_entry_is_packed_only_when_requested() {
        let mut cfg = config(None);
        let packed = |cfg: &PackConfig| {
            let mut entries = Vec::new();
            if let Some(flag) = tynd_pack_format::debug_flag_entry(cfg.debug) {
                entries.push(PackEntry::raw(flag.rel, flag.data.to_vec()));
            }
            raw_entries(&entries)
                .iter()
                .any(|(rel, data)| *rel == tynd_pack_format::DEBUG_FLAG_ENTRY && *data == b"1")
        };
        assert!(!packed(&cfg));
        cfg.debug = true;
        assert!(packed(&cfg));
    }
}
