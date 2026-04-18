//! Shared TYNDPKG reader used by both `tynd-full` and `tynd-lite`.
//!
//! Wire format (written by `packages/cli/src/bundle/pack.ts`):
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
//! This module owns: magic trailer detection, entry header parsing,
//! temp-dir layout, and sidecar finalization. Callers (`full`, `lite`)
//! own the per-entry dispatch (different entry keys, different output
//! layout) and consume bytes by reading `data_len` from `reader()`.

use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

pub const MAGIC: &[u8; 8] = b"TYNDPKG\0";
/// section_size(u64) + magic(8) = 16 bytes
pub const TRAILER_LEN: u64 = 16;

#[derive(Debug)]
pub struct PackReader {
    file: File,
    remaining: usize,
}

impl PackReader {
    /// Open the currently running executable and locate the packed section.
    /// Returns `None` if no TYNDPKG magic trailer is present.
    pub fn open() -> Option<Self> {
        let exe = std::env::current_exe().ok()?;
        let mut f = File::open(&exe).ok()?;
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

        Some(Self {
            file: f,
            remaining: file_count,
        })
    }

    /// Number of entries still to read.
    pub fn remaining(&self) -> usize {
        self.remaining
    }

    /// Read the next entry header. Returns `None` when the pack is exhausted.
    /// Caller consumes exactly `data_len` bytes via `reader()` before calling
    /// `next_header()` again.
    pub fn next_header(&mut self) -> Option<(String, u64)> {
        if self.remaining == 0 {
            return None;
        }
        self.remaining -= 1;

        let mut pl = [0u8; 2];
        self.file.read_exact(&mut pl).ok()?;
        let path_len = u16::from_le_bytes(pl) as usize;
        let mut path_buf = vec![0u8; path_len];
        self.file.read_exact(&mut path_buf).ok()?;
        let rel = String::from_utf8(path_buf).ok()?;

        let mut dl = [0u8; 4];
        self.file.read_exact(&mut dl).ok()?;
        let data_len = u32::from_le_bytes(dl) as u64;

        Some((rel, data_len))
    }

    /// Borrow the underlying file for reading the current entry's bytes.
    pub fn reader(&mut self) -> &mut File {
        &mut self.file
    }

    /// Discard the next `data_len` bytes without allocating — used when an
    /// entry key is unknown or intentionally skipped.
    pub fn skip(&mut self, data_len: u64) -> io::Result<()> {
        let mut src = (&mut self.file).take(data_len);
        io::copy(&mut src, &mut io::sink())?;
        Ok(())
    }
}

/// Layout of the extraction target: a registered temp dir with pre-created
/// `backend/` and `frontend/` subdirectories.
#[derive(Debug)]
pub struct ExtractPaths {
    pub temp_dir: PathBuf,
    pub bundle_dir: PathBuf,
    pub frontend_dir: PathBuf,
}

/// Create a `tynd-*` temp directory, register it for cleanup, and prepare
/// the standard subfolders. The `TempDir` handle is leaked on purpose — our
/// cleanup runs on `process::exit` paths that bypass Rust's drop machinery.
pub fn prepare_extract_dir() -> Option<ExtractPaths> {
    let td = tempfile::TempDir::with_prefix("tynd-").ok()?;
    let temp_dir = td.path().to_owned();
    #[allow(clippy::mem_forget)]
    std::mem::forget(td);
    crate::cleanup::register_dir(temp_dir.clone());

    let bundle_dir = temp_dir.join("backend");
    let frontend_dir = temp_dir.join("frontend");
    fs::create_dir_all(&bundle_dir).ok()?;
    fs::create_dir_all(&frontend_dir).ok()?;

    Some(ExtractPaths {
        temp_dir,
        bundle_dir,
        frontend_dir,
    })
}

/// chmod +755 each extracted sidecar (Unix) and register its on-disk path
/// so app code can resolve it via `os::sidecar::path(name)`.
pub fn finalize_sidecars(pending: &[(String, PathBuf)]) {
    for (name, path) in pending {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o755));
        }
        crate::os::sidecar::register(name, &path.to_string_lossy());
    }
}

/// Best-effort app stem for cache path derivation (full mode only).
pub fn exe_stem(exe: &Path) -> String {
    exe.file_stem()
        .map_or_else(|| "app".to_string(), |s| s.to_string_lossy().into_owned())
}
