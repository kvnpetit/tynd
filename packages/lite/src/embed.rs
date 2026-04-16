//! Embedded-assets support for `tynd build` (lite runtime).
//!
//! `tynd build` appends a packed section to the tynd-lite binary:
//!
//! ```text
//! [tynd-lite binary]
//! ┌─ packed section ──────────────────────────────────────────────────┐
//! │  file_count : u32 LE                                              │
//! │  for each file:                                                   │
//! │    path_len : u16 LE                                              │
//! │    path     : UTF-8 bytes (relative: "bundle.js" or "frontend/…") │
//! │    data_len : u32 LE                                              │
//! │    data     : raw bytes                                           │
//! └───────────────────────────────────────────────────────────────────┘
//! section_size : u64 LE   (byte count of the packed section above)
//! magic        : "TYNDPKG\0" (8 bytes)
//! ```
//!
//! At startup, `try_load_embedded()` checks for the magic trailer.
//! If found, it extracts all files to a temporary directory and returns
//! the paths that tynd-lite needs.

use std::fs;
use std::io::{BufWriter, Read, Seek, SeekFrom, Write};
use std::path::PathBuf;

const MAGIC: &[u8; 8] = b"TYNDPKG\0";
/// section_size(u64) + magic(8) = 16 bytes
const TRAILER_LEN: u64 = 16;

pub(crate) struct EmbeddedAssets {
    pub bundle_path: String,
    pub frontend_dir: String,
    /// Optional icon file extracted from the pack (PNG or ICO).
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
    // Cleanup is instead registered with tynd_host::cleanup so it runs
    // before every process::exit (which bypasses Rust's drop machinery).
    let td = tempfile::TempDir::with_prefix("tynd-").ok()?;
    let temp_dir = td.path().to_owned();
    #[allow(clippy::mem_forget)] // intentional: cleanup::run() handles removal across exit paths
    std::mem::forget(td);
    tynd_host::cleanup::register_dir(temp_dir.clone());
    let bundle_dir = temp_dir.join("backend");
    let frontend_dir = temp_dir.join("frontend");
    fs::create_dir_all(&bundle_dir).ok()?;
    fs::create_dir_all(&frontend_dir).ok()?;

    let mut bundle_path_opt: Option<String> = None;
    let mut frontend_dir_opt: Option<String> = None;
    let mut icon_path_opt: Option<String> = None;

    for _ in 0..file_count {
        // path
        let mut pl = [0u8; 2];
        f.read_exact(&mut pl).ok()?;
        let path_len = u16::from_le_bytes(pl) as usize;
        let mut path_buf = vec![0u8; path_len];
        f.read_exact(&mut path_buf).ok()?;
        let rel = String::from_utf8(path_buf).ok()?;

        // data length
        let mut dl = [0u8; 4];
        f.read_exact(&mut dl).ok()?;
        let data_len = u32::from_le_bytes(dl) as u64;

        let dest: PathBuf = if rel == "bundle.js" {
            let p = bundle_dir.join("bundle.js");
            bundle_path_opt = Some(p.to_string_lossy().into_owned());
            p
        } else if let Some(rest) = rel.strip_prefix("frontend/") {
            let p = frontend_dir.join(rest);
            if let Some(parent) = p.parent() {
                let _ = fs::create_dir_all(parent);
            }
            if frontend_dir_opt.is_none() {
                frontend_dir_opt = Some(frontend_dir.to_string_lossy().into_owned());
            }
            p
        } else if rel == "icon.png" || rel == "icon.ico" {
            let p = temp_dir.join(&rel);
            icon_path_opt = Some(p.to_string_lossy().into_owned());
            p
        } else {
            // Unknown path — skip its bytes and continue
            let mut skip = (&mut f).take(data_len);
            std::io::copy(&mut skip, &mut std::io::sink()).ok()?;
            continue;
        };

        // Stream directly from the exe into the temp file — no full-file allocation.
        let out = fs::File::create(&dest).ok()?;
        let mut writer = BufWriter::new(out);
        std::io::copy(&mut (&mut f).take(data_len), &mut writer).ok()?;
        writer.flush().ok()?;
    }

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

    Some(EmbeddedAssets {
        bundle_path,
        frontend_dir,
        icon_path: icon_path_opt,
    })
}
