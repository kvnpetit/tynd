//! TYNDPKG reader used by the Tynd host.
//!
//! Wire format (written by `tynd-bundler` and shared through
//! `tynd-pack-format`):
//!
//! ```text
//! [host binary]
//! ┌─ packed section ──────────────────────────────────────────────────┐
//! │  file_count : u32 LE                                              │
//! │  for each file:                                                   │
//! │    path_len : u16 LE                                              │
//! │    path     : UTF-8 bytes                                         │
//! │    data_len : u32 LE                                              │
//! │    data     : raw bytes                                           │
//! └───────────────────────────────────────────────────────────────────┘
//! section_size : u64 LE
//! magic        : "TYNDPKG\0" (8 bytes)
//! ```
//!
//! `tynd-pack-format` owns trailer/header parsing and exact entry consumption.
//! This facade owns temp-dir layout and sidecar finalization; the runtime owns
//! per-entry dispatch.

use std::{
    fs,
    path::{Path, PathBuf},
    sync::OnceLock,
};

pub use tynd_pack_format::PackReader;

/// Set once, only when running from an embedded pack: whether that pack was
/// built with the debug flag (`tynd build --debug-protocol`). Unset means an
/// unpacked run (`tynd dev` / `start`).
static PACK_DEBUG: OnceLock<bool> = OnceLock::new();

/// Record that this process runs from an embedded pack and whether the pack
/// allows the debug protocol. Call before the host starts.
pub fn set_pack_debug(allowed: bool) {
    let _ = PACK_DEBUG.set(allowed);
}

/// `None` for an unpacked run, else the packed debug flag.
#[cfg(feature = "debug-protocol")]
pub(crate) fn pack_debug_policy() -> Option<bool> {
    PACK_DEBUG.get().copied()
}

/// Layout of the extraction target: a registered temp dir with pre-created
/// `backend/` and `frontend/` subdirectories.
#[derive(Debug)]
pub struct ExtractPaths {
    pub root: PathBuf,
    pub backend: PathBuf,
    pub frontend: PathBuf,
}

/// Create a `tynd-*` temp directory, register it for cleanup, and prepare
/// the standard subfolders. Automatic deletion is disabled because explicit
/// cleanup also covers `process::exit` paths that bypass Rust drops.
pub fn prepare_extract_dir() -> Option<ExtractPaths> {
    let temp_dir = tempfile::TempDir::with_prefix("tynd-").ok()?.keep();
    crate::cleanup::register_dir(temp_dir.clone());

    let bundle_dir = temp_dir.join("backend");
    let frontend_dir = temp_dir.join("frontend");
    fs::create_dir_all(&bundle_dir).ok()?;
    fs::create_dir_all(&frontend_dir).ok()?;

    Some(ExtractPaths {
        root: temp_dir,
        backend: bundle_dir,
        frontend: frontend_dir,
    })
}

/// chmod +755 each extracted sidecar (Unix) and register its on-disk path
/// so app code can resolve it via `api::sidecar::path(name)`.
///
/// When `api-sidecar` isn't compiled in the registration is skipped — the
/// files are still extracted, they just aren't discoverable via the API.
/// Custom builds that drop the feature shouldn't pack sidecars in the first
/// place; this is belt-and-suspenders.
pub fn finalize_sidecars(pending: &[(String, PathBuf)]) {
    for (name, path) in pending {
        #[cfg(all(not(unix), not(feature = "api-sidecar")))]
        let _ = (name, path);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Err(e) = fs::set_permissions(path, fs::Permissions::from_mode(0o755)) {
                // Without +x the sidecar is unrunnable; drop it so the next
                // extract pass re-creates a usable binary.
                crate::tynd_error!(
                    "sidecar chmod failed for {name} ({path}): {e}; removing partial cache",
                    name,
                    path = path.display()
                );
                let _ = fs::remove_file(path);
                // Without the sidecar API there is no work left in this
                // iteration; with it, skip registration of the removed file.
                #[cfg(feature = "api-sidecar")]
                continue;
            }
        }
        #[cfg(feature = "api-sidecar")]
        crate::api::sidecar::register(name, &path.to_string_lossy());
    }
}

/// Best-effort app stem for cache path derivation (full mode only).
pub fn exe_stem(exe: &Path) -> String {
    exe.file_stem()
        .map_or_else(|| "app".to_string(), |s| s.to_string_lossy().into_owned())
}
