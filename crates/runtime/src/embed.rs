//! Embedded-assets loader for the Bun runtime.
//!
//! Pack entries (order matters — Bun metadata must precede `bun.zst`):
//! `bun.version`, `bun.blake3`, `bun.zst`, `webview2.runtime-path`,
//! `observability.json`, `debug.flag` (optional), `bundle.js`, `frontend/<path>`, `icon.{png,ico}`, `sidecar/<name>`. See
//! `tynd_host::embed` for the wire format.

use std::{
    collections::HashSet,
    fs::{self, File, OpenOptions},
    io::{BufReader, BufWriter, Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use tynd_host::embed::{
    exe_stem, finalize_sidecars, prepare_extract_dir, ExtractPaths, PackReader,
};

pub(crate) struct EmbeddedAssets {
    /// Path to the integrity-checked cached Bun binary.
    pub bun_path: String,
    pub bundle_path: String,
    pub frontend_dir: String,
    pub icon_path: Option<String>,
    pub webview2_runtime_path: Option<String>,
    /// Raw `observability.json` entry (see `crate::observability`), if packed.
    pub observability: Option<String>,
    /// The pack carries the debug flag (`tynd build --debug-protocol`).
    pub debug: bool,
}

/// Whether this executable carries a `TYNDPKG` section (damaged or not).
/// Cheap: reads only the trailer, extracts nothing.
pub(crate) fn is_packaged() -> bool {
    !matches!(PackReader::open_checked(), Ok(None))
}

/// `None` means the executable has no pack (dev run). Once a pack is detected
/// every failure is fatal: falling back to dev arguments would hide the real
/// problem behind a misleading `--backend-entry is required`.
pub(crate) fn try_load_embedded() -> Option<EmbeddedAssets> {
    let mut pack = match PackReader::open_checked() {
        Ok(None) => return None,
        Ok(Some(pack)) => pack,
        Err(error) => fatal_embedded(&format!("the embedded pack is damaged ({error})")),
    };
    let exe = std::env::current_exe()
        .unwrap_or_else(|error| fatal_embedded(&format!("cannot locate the executable ({error})")));
    let Some(paths) = prepare_extract_dir() else {
        fatal_embedded("cannot create a temporary extraction directory; check free disk space")
    };
    let mut state = EmbeddedLoadState::new(exe_stem(&exe), paths);

    loop {
        match next_pack_header(&mut pack) {
            Ok(Some((relative_path, data_len))) => {
                if state
                    .consume_entry(&mut pack, relative_path.clone(), data_len)
                    .is_none()
                {
                    fatal_embedded(&format!("cannot extract '{relative_path}'"));
                }
            },
            Ok(None) => break,
            Err(()) => fatal_embedded("the embedded pack is malformed"),
        }
    }

    Some(state.finish())
}

fn fatal_embedded(reason: &str) -> ! {
    tynd_host::tynd_error!(
        "Cannot start: {reason}. Reinstall the application, and check that the user cache and \
         temp directories are writable and not full."
    );
    tynd_host::cleanup();
    std::process::exit(1)
}

fn next_pack_header(pack: &mut PackReader) -> Result<Option<(String, u64)>, ()> {
    pack.next_header().map_err(|error| {
        tynd_host::tynd_error!("Embedded pack is malformed: {error}");
    })
}

struct EmbeddedLoadState {
    app_name: String,
    paths: ExtractPaths,
    bun_version: Option<String>,
    bun_hash: Option<String>,
    bun_path: Option<PathBuf>,
    bundle_path: Option<String>,
    frontend_dir: Option<String>,
    icon_path: Option<String>,
    webview2_runtime_path: Option<String>,
    observability: Option<String>,
    debug: bool,
    sidecars: Vec<(String, PathBuf)>,
    seen_entries: HashSet<String>,
    packed_bytes: u64,
}

impl EmbeddedLoadState {
    fn new(app_name: String, paths: ExtractPaths) -> Self {
        Self {
            app_name,
            paths,
            bun_version: None,
            bun_hash: None,
            bun_path: None,
            bundle_path: None,
            frontend_dir: None,
            icon_path: None,
            webview2_runtime_path: None,
            observability: None,
            debug: false,
            sidecars: Vec::new(),
            seen_entries: HashSet::new(),
            packed_bytes: 0,
        }
    }

    fn consume_entry(
        &mut self,
        pack: &mut PackReader,
        relative_path: String,
        data_len: u64,
    ) -> Option<()> {
        if !self.seen_entries.insert(relative_path.clone()) {
            tynd_host::tynd_error!("Embedded pack contains duplicate entry '{relative_path}'");
            return None;
        }
        if !account_pack_entry(&relative_path, data_len, &mut self.packed_bytes) {
            pack.skip(data_len).ok()?;
            return None;
        }

        match relative_path.as_str() {
            "bun.version" => self.read_bun_version(pack, data_len),
            "bun.blake3" => self.read_bun_hash(pack, data_len),
            "bun.zst" => self.read_bun_archive(pack, data_len),
            tynd_pack_format::WEBVIEW2_RUNTIME_PATH_ENTRY => {
                self.read_webview2_runtime_path(pack, data_len)
            },
            crate::observability::PACK_ENTRY => self.read_observability(pack, data_len),
            tynd_pack_format::DEBUG_FLAG_ENTRY => self.read_debug_flag(pack, data_len),
            "bundle.js" => self.write_bundle(pack, data_len),
            "icon.ico" | "icon.png" => self.write_icon(pack, &relative_path, data_len),
            _ if relative_path.starts_with("frontend/") => {
                self.write_frontend(pack, &relative_path, data_len)
            },
            _ if relative_path.starts_with("sidecar/") => {
                self.write_sidecar(pack, &relative_path, data_len)
            },
            _ => pack.skip(data_len).ok(),
        }
    }

    fn read_bun_version(&mut self, pack: &mut PackReader, data_len: u64) -> Option<()> {
        if data_len > 256 {
            tynd_host::tynd_warn!(
                "Embedded bun.version entry is suspiciously large ({data_len} bytes) — skipping"
            );
            pack.skip(data_len).ok()?;
            return Some(());
        }
        let version = read_small_text_entry(pack, data_len)?;
        if !valid_bun_version(&version) {
            tynd_host::tynd_error!("Embedded bun.version is invalid");
            return None;
        }
        self.bun_version = Some(version);
        Some(())
    }

    fn read_bun_hash(&mut self, pack: &mut PackReader, data_len: u64) -> Option<()> {
        if data_len > 128 {
            tynd_host::tynd_error!("Embedded bun.blake3 is suspiciously large");
            pack.skip(data_len).ok()?;
            return None;
        }
        let hash = read_small_text_entry(pack, data_len)?.to_ascii_lowercase();
        if !valid_blake3(&hash) {
            tynd_host::tynd_error!("Embedded bun.blake3 is invalid");
            return None;
        }
        self.bun_hash = Some(hash);
        Some(())
    }

    fn read_bun_archive(&mut self, pack: &mut PackReader, data_len: u64) -> Option<()> {
        let (Some(version), Some(expected_hash)) =
            (self.bun_version.as_deref(), self.bun_hash.as_deref())
        else {
            tynd_host::tynd_error!(
                "Embedded bun.zst is missing preceding bun.version or bun.blake3 metadata"
            );
            pack.skip(data_len).ok()?;
            return None;
        };
        let cache_path = bun_cache_path(&self.app_name, version, expected_hash);
        if !extract_bun(pack, data_len, &cache_path, expected_hash) {
            return None;
        }
        self.bun_path = Some(cache_path);
        Some(())
    }

    fn read_webview2_runtime_path(&mut self, pack: &mut PackReader, data_len: u64) -> Option<()> {
        if data_len == 0 || data_len > tynd_pack_format::MAX_BOOT_METADATA_BYTES {
            tynd_host::tynd_error!(
                "Embedded WebView2 runtime path has invalid size ({data_len} bytes)"
            );
            pack.skip(data_len).ok()?;
            return None;
        }
        let path = read_small_text_entry(pack, data_len)?;
        if safe_subpath(&path).is_none() {
            tynd_host::tynd_error!("Embedded WebView2 runtime path is unsafe");
            return None;
        }
        self.webview2_runtime_path = Some(path);
        Some(())
    }

    /// Content is validated later by `crate::observability` (invalid JSON
    /// disables observability but never blocks boot); size is capped by
    /// `account_pack_entry`.
    fn read_observability(&mut self, pack: &mut PackReader, data_len: u64) -> Option<()> {
        self.observability = Some(read_small_text_entry(pack, data_len)?);
        Some(())
    }

    /// Anything but a well-formed "on" payload leaves the debug server disabled.
    fn read_debug_flag(&mut self, pack: &mut PackReader, data_len: u64) -> Option<()> {
        let mut bytes = Vec::with_capacity(data_len as usize);
        pack.copy_entry_to(data_len, &mut bytes).ok()?;
        self.debug = tynd_pack_format::parse_debug_flag(&bytes);
        Some(())
    }

    fn write_bundle(&mut self, pack: &mut PackReader, data_len: u64) -> Option<()> {
        let destination = self.paths.backend.join("bundle.js");
        write_entry(pack, data_len, &destination)?;
        self.bundle_path = Some(destination.to_string_lossy().into_owned());
        Some(())
    }

    fn write_icon(
        &mut self,
        pack: &mut PackReader,
        relative_path: &str,
        data_len: u64,
    ) -> Option<()> {
        let destination = self.paths.root.join(relative_path);
        write_entry(pack, data_len, &destination)?;
        self.icon_path = Some(destination.to_string_lossy().into_owned());
        Some(())
    }

    fn write_frontend(
        &mut self,
        pack: &mut PackReader,
        relative_path: &str,
        data_len: u64,
    ) -> Option<()> {
        let rest = &relative_path["frontend/".len()..];
        let Some(safe) = safe_subpath(rest) else {
            pack.skip(data_len).ok()?;
            return Some(());
        };
        let destination = self.paths.frontend.join(safe);
        create_parent_directory(&destination);
        write_entry(pack, data_len, &destination)?;
        self.frontend_dir
            .get_or_insert_with(|| self.paths.frontend.to_string_lossy().into_owned());
        Some(())
    }

    fn write_sidecar(
        &mut self,
        pack: &mut PackReader,
        relative_path: &str,
        data_len: u64,
    ) -> Option<()> {
        let rest = &relative_path["sidecar/".len()..];
        let Some(safe) = safe_subpath(rest) else {
            pack.skip(data_len).ok()?;
            return Some(());
        };
        let destination = self.paths.root.join("sidecar").join(safe);
        create_parent_directory(&destination);
        write_entry(pack, data_len, &destination)?;
        self.sidecars.push((safe.to_string(), destination));
        Some(())
    }

    fn finish(self) -> EmbeddedAssets {
        let bun_path = self.bun_path.filter(|path| path.is_file()).map_or_else(
            || missing_required_asset("a valid Bun runtime"),
            |path| path.to_string_lossy().into_owned(),
        );
        let bundle_path = self
            .bundle_path
            .unwrap_or_else(|| missing_required_asset("bundle.js"));
        let frontend_dir = self
            .frontend_dir
            .unwrap_or_else(|| missing_required_asset("frontend files"));
        finalize_sidecars(&self.sidecars);

        EmbeddedAssets {
            bun_path,
            bundle_path,
            frontend_dir,
            icon_path: self.icon_path,
            webview2_runtime_path: self.webview2_runtime_path,
            observability: self.observability,
            debug: self.debug,
        }
    }
}

fn create_parent_directory(path: &Path) {
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
}

fn missing_required_asset(name: &str) -> ! {
    tynd_host::tynd_error!("Embedded pack is missing {name} — rebuild with `tynd build`");
    tynd_host::cleanup();
    std::process::exit(1)
}
const MAX_PACK_ENTRY_BYTES: u64 = 512 * 1024 * 1024;
const MAX_PACK_TOTAL_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const MAX_BACKEND_BUNDLE_BYTES: u64 = 128 * 1024 * 1024;
const MAX_ICON_BYTES: u64 = 32 * 1024 * 1024;
const MAX_FRONTEND_ENTRY_BYTES: u64 = 128 * 1024 * 1024;

fn account_pack_entry(rel: &str, data_len: u64, total: &mut u64) -> bool {
    let entry_cap = if rel == tynd_pack_format::WEBVIEW2_RUNTIME_PATH_ENTRY
        || rel == tynd_pack_format::DEBUG_FLAG_ENTRY
        || rel == crate::observability::PACK_ENTRY
    {
        tynd_pack_format::MAX_BOOT_METADATA_BYTES
    } else if rel == "bundle.js" {
        MAX_BACKEND_BUNDLE_BYTES
    } else if rel == "icon.ico" || rel == "icon.png" {
        MAX_ICON_BYTES
    } else if rel.starts_with("frontend/") {
        MAX_FRONTEND_ENTRY_BYTES
    } else {
        MAX_PACK_ENTRY_BYTES
    };
    if data_len > entry_cap {
        tynd_host::tynd_error!(
            "Embedded pack entry '{rel}' is {data_len} bytes; limit is {entry_cap} bytes"
        );
        return false;
    }
    let Some(next_total) = total.checked_add(data_len) else {
        tynd_host::tynd_error!("Embedded pack size overflow");
        return false;
    };
    if next_total > MAX_PACK_TOTAL_BYTES {
        tynd_host::tynd_error!("Embedded pack entries exceed {MAX_PACK_TOTAL_BYTES} total bytes");
        return false;
    }
    *total = next_total;
    true
}

// Reject `../`, `./`, absolute paths, Windows drive prefixes, UNC
// paths, empty / separator-bearing components so a malformed pack
// can't write outside the extraction dir.
fn safe_subpath(rest: &str) -> Option<&str> {
    tynd_pack_format::validate_subpath(rest)
}

pub(crate) fn resolve_webview2_runtime_path(exe: &Path, relative: &str) -> Option<PathBuf> {
    safe_subpath(relative)?;
    let runtime = exe.parent()?.join(relative);
    runtime.is_dir().then_some(runtime)
}

fn write_entry(pack: &mut PackReader, data_len: u64, dest: &Path) -> Option<()> {
    let out = File::create(dest)
        .map_err(|error| tynd_host::tynd_error!("Cannot create {}: {error}", dest.display()))
        .ok()?;
    let mut writer = BufWriter::new(out);
    pack.copy_entry_to(data_len, &mut writer)
        .and_then(|_| writer.flush())
        .map_err(|error| tynd_host::tynd_error!("Cannot write {}: {error}", dest.display()))
        .ok()?;
    Some(())
}

const MAX_COMPRESSED_BUN_BYTES: u64 = 512 * 1024 * 1024;
const MAX_DECOMPRESSED_BUN_BYTES: u64 = 1024 * 1024 * 1024;

fn read_small_text_entry(pack: &mut PackReader, data_len: u64) -> Option<String> {
    let mut bytes = Vec::with_capacity(data_len as usize);
    pack.copy_entry_to(data_len, &mut bytes).ok()?;
    String::from_utf8(bytes)
        .ok()
        .map(|value| value.trim().to_string())
}

fn valid_bun_version(version: &str) -> bool {
    !version.is_empty()
        && version.len() <= 64
        && version != "."
        && version != ".."
        && version
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b".-+_".contains(&byte))
}

fn valid_blake3(hash: &str) -> bool {
    hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn blake3_file(path: &Path) -> std::io::Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = blake3::Hasher::new();
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher.finalize().to_hex().to_string())
}

fn cached_bun_is_valid(cache_path: &Path, expected_hash: &str) -> bool {
    let Ok(metadata) = fs::symlink_metadata(cache_path) else {
        return false;
    };
    metadata.file_type().is_file()
        && metadata.len() > 0
        && metadata.len() <= MAX_DECOMPRESSED_BUN_BYTES
        && blake3_file(cache_path).is_ok_and(|actual| actual == expected_hash)
}

fn unique_sibling_path(cache_path: &Path, kind: &str) -> PathBuf {
    static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);
    let id = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
    let name = cache_path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("bun");
    cache_path.with_file_name(format!(".{name}.{kind}-{}-{id}", std::process::id()))
}

fn extract_bun(
    pack: &mut PackReader,
    data_len: u64,
    cache_path: &Path,
    expected_hash: &str,
) -> bool {
    if data_len == 0 || data_len > MAX_COMPRESSED_BUN_BYTES {
        tynd_host::tynd_error!("Embedded bun.zst size {data_len} is outside the supported range");
        let _ = pack.skip(data_len);
        return false;
    }
    if cached_bun_is_valid(cache_path, expected_hash) {
        return pack.skip(data_len).is_ok();
    }
    if let Err(message) = clear_invalid_cached_bun(cache_path) {
        tynd_host::tynd_error!("{message}");
        let _ = pack.skip(data_len);
        return false;
    }

    eprintln!("First launch — extracting runtime…");
    if let Some(parent) = cache_path.parent() {
        if let Err(error) = fs::create_dir_all(parent) {
            tynd_host::tynd_error!("Failed to create Bun cache dir: {error}");
            let _ = pack.skip(data_len);
            return false;
        }
    }

    let compressed_path = unique_sibling_path(cache_path, "compressed");
    let output_path = unique_sibling_path(cache_path, "extracting");
    if let Err(message) = spool_compressed_bun(pack, data_len, &compressed_path) {
        tynd_host::tynd_error!("{message}");
        return false;
    }

    let result = extract_bun_archive(&compressed_path, &output_path, expected_hash)
        .and_then(|()| publish_extracted_bun(&output_path, cache_path, expected_hash));
    let _ = fs::remove_file(&compressed_path);
    let _ = fs::remove_file(&output_path);

    match result {
        Ok(()) => {
            eprintln!("Runtime ready.");
            true
        },
        Err(error) => {
            tynd_host::tynd_error!("Failed to extract embedded Bun runtime: {error}");
            false
        },
    }
}

/// Remove a cached runtime that failed validation so a fresh copy can be
/// published. A missing cache entry is fine; anything that is not a regular
/// file or symlink is left alone and reported as an error.
fn clear_invalid_cached_bun(cache_path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(cache_path) {
        Ok(metadata) if metadata.file_type().is_file() || metadata.file_type().is_symlink() => {
            fs::remove_file(cache_path)
                .map_err(|error| format!("Failed to remove invalid cached Bun runtime: {error}"))
        },
        Ok(_) => Err("Bun cache path exists but is not a regular file".to_string()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("Failed to inspect Bun cache path: {error}")),
    }
}

/// Copy the `bun.zst` pack entry into a fresh temporary file at
/// `compressed_path` and flush it. The temporary file is removed again on
/// failure; the handle is closed before returning so it can be read back.
fn spool_compressed_bun(
    pack: &mut PackReader,
    data_len: u64,
    compressed_path: &Path,
) -> Result<(), String> {
    let compressed = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(compressed_path);
    let Ok(mut compressed) = compressed else {
        let _ = pack.skip(data_len);
        return Err("Failed to create temporary Bun archive".to_string());
    };
    let written = pack
        .copy_entry_to(data_len, &mut compressed)
        .map_err(|error| format!("Failed to read bun.zst from pack: {error}"))
        .and_then(|_copied| {
            compressed
                .sync_all()
                .map_err(|error| format!("Failed to flush temporary Bun archive: {error}"))
        });
    if written.is_err() {
        let _ = fs::remove_file(compressed_path);
    }
    written
}

/// Publish the verified temporary runtime at `cache_path` without ever
/// exposing a partial file. A hard link is preferred because it never replaces
/// a runtime a concurrent launch already published; filesystems without hard
/// links (FAT/exFAT, some redirected profiles) fall back to an atomic rename
/// inside the same directory.
fn publish_extracted_bun(
    output_path: &Path,
    cache_path: &Path,
    expected_hash: &str,
) -> Result<(), String> {
    let published_by_other = || cached_bun_is_valid(cache_path, expected_hash);
    match fs::hard_link(output_path, cache_path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            if published_by_other() {
                Ok(())
            } else {
                Err(format!("publish extracted Bun runtime: {error}"))
            }
        },
        Err(link_error) => fs::rename(output_path, cache_path).or_else(|rename_error| {
            if published_by_other() {
                Ok(())
            } else {
                Err(format!(
                    "publish extracted Bun runtime: hard link failed ({link_error}), rename \
                     failed ({rename_error})"
                ))
            }
        }),
    }
}

fn extract_bun_archive(
    compressed_path: &Path,
    output_path: &Path,
    expected_hash: &str,
) -> Result<(), String> {
    let compressed = File::open(compressed_path)
        .map_err(|error| format!("open temporary Bun archive: {error}"))?;
    let mut decoder = zstd::Decoder::new(BufReader::new(compressed))
        .map_err(|error| format!("initialize zstd decoder: {error}"))?;
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output_path)
        .map_err(|error| format!("create temporary Bun runtime: {error}"))?;
    let mut hasher = blake3::Hasher::new();
    let mut total = 0u64;
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        let read = decoder
            .read(&mut buffer)
            .map_err(|error| format!("decompress Bun runtime: {error}"))?;
        if read == 0 {
            break;
        }
        total = total
            .checked_add(read as u64)
            .ok_or_else(|| "decompressed Bun size overflow".to_string())?;
        if total > MAX_DECOMPRESSED_BUN_BYTES {
            return Err(format!(
                "decompressed Bun exceeds {MAX_DECOMPRESSED_BUN_BYTES} byte limit"
            ));
        }
        hasher.update(&buffer[..read]);
        output
            .write_all(&buffer[..read])
            .map_err(|error| format!("write temporary Bun runtime: {error}"))?;
    }
    if total == 0 {
        return Err("decompressed Bun runtime is empty".into());
    }
    output
        .sync_all()
        .map_err(|error| format!("flush temporary Bun runtime: {error}"))?;
    let actual_hash = hasher.finalize().to_hex().to_string();
    if actual_hash != expected_hash {
        return Err(format!(
            "BLAKE3 mismatch: expected {expected_hash}, got {actual_hash}"
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(output_path, fs::Permissions::from_mode(0o755))
            .map_err(|error| format!("mark temporary Bun runtime executable: {error}"))?;
    }
    Ok(())
}

/// Returns the persistent cache path for the Bun binary.
///
/// - Windows: `%LOCALAPPDATA%\tynd\{app}\bun-{ver}-{hash}\bun.exe`
/// - macOS:   `~/Library/Caches/tynd/{app}/bun-{ver}-{hash}/bun`
/// - Linux:   `~/.cache/tynd/{app}/bun-{ver}-{hash}/bun`
fn bun_cache_path(app_name: &str, version: &str, expected_hash: &str) -> PathBuf {
    let base = dirs::cache_dir().unwrap_or_else(std::env::temp_dir);
    let bin_name = if cfg!(windows) {
        format!("{app_name}.exe")
    } else {
        app_name.to_string()
    };
    base.join("tynd")
        .join(app_name)
        .join(format!("bun-{version}-{}", &expected_hash[..12]))
        .join(bin_name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tynd-embed-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn publishes_a_verified_runtime() {
        let dir = temp_dir("publish");
        let (output, cache) = (dir.join("tmp"), dir.join("bun"));
        fs::write(&output, b"runtime").unwrap();
        let hash = blake3::hash(b"runtime").to_hex().to_string();
        publish_extracted_bun(&output, &cache, &hash).unwrap();
        assert_eq!(fs::read(&cache).unwrap(), b"runtime");
        // A second launch finding the same valid runtime is not an error.
        publish_extracted_bun(&output, &cache, &hash).unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn refuses_to_publish_over_a_different_file() {
        let dir = temp_dir("conflict");
        let (output, cache) = (dir.join("tmp"), dir.join("bun"));
        fs::write(&output, b"runtime").unwrap();
        fs::write(&cache, b"other").unwrap();
        let hash = blake3::hash(b"runtime").to_hex().to_string();
        assert!(publish_extracted_bun(&output, &cache, &hash).is_err());
        let _ = fs::remove_dir_all(&dir);
    }
}
