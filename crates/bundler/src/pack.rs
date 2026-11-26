//! `tynd-bundler pack` — produce a self-extracting `.exe` (TYNDPKG).
//!
//! Wire format (must stay byte-for-byte identical with what
//! `tynd-pack-format` and `tynd-runtime/src/embed.rs` read):
//!
//! ```text
//! [host binary bytes]
//! [section]
//!   file_count : u32 LE
//!   for each entry:
//!     path_len : u16 LE
//!     path     : UTF-8 bytes
//!     data_len : u32 LE
//!     data     : raw bytes
//! [trailer]
//!   section_size : u64 LE   (size of [section], excluding trailer)
//!   magic        : "TYNDPKG\0" (8 bytes)
//! ```
//!
//! Text assets (`.html .htm .js .mjs .cjs .css .json .svg`) get auto-zstd'd
//! at level 19 and packed under `<original-path>.zst`; the host
//! scheme.rs handler streams them through a decoder transparently.

use std::{
    fmt,
    fs::{self, File},
    io::Read,
    path::Path,
};

use anyhow::Result;
use error_stack::{Report, ResultExt as _};
use miette::Diagnostic;
use tempfile::{Builder as TempFileBuilder, TempPath};
use thiserror::Error;
use tracing::{debug, info};
use tynd_bundle_contract::{PackConfig, SidecarSpec};

use crate::pe;

mod bun_cache;
mod container;
pub(crate) mod icons;

use container::{collect_entries, write_packed, PackEntry};
use icons::build_ico_bytes;

/// `error-stack` context markers for the pack pipeline. Rendered into the
/// `cause` field of `PackError::PePatch` / `::IconRender` / `::Io` at the
/// boundary so the diagnostic carries the host path / icon source / asset
/// sizes instead of the bare upstream `Display`.
#[derive(Debug)]
pub(crate) enum PackContext {
    PatchSubsystem,
    EmbedIconRsrc,
    CompressBunRuntime,
    AssembleIco,
    WritePackedFile,
}

impl fmt::Display for PackContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PatchSubsystem => f.write_str("patch PE subsystem to GUI"),
            Self::EmbedIconRsrc => f.write_str("embed ICO + RT_VERSION into PE rsrc"),
            Self::CompressBunRuntime => f.write_str("zstd-compress bun runtime"),
            Self::AssembleIco => f.write_str("assemble multi-size ICO"),
            Self::WritePackedFile => f.write_str("write TYNDPKG self-extracting exe"),
        }
    }
}

impl std::error::Error for PackContext {}

const MAX_HOST_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Debug, Clone, Error, Diagnostic)]
pub(crate) enum PackError {
    #[error("host binary not found: {path}")]
    #[diagnostic(
        code(tynd::bundler::pack::missing_host),
        url("https://kvnpetit.github.io/tynd/docs/v1.0/concepts/tyndpkg"),
        help(
            "the `host_binary` field must point at a built `tynd-runtime.exe` (or \
             platform equivalent). Run the relevant cargo build step first, then re-run `tynd \
             build`."
        )
    )]
    MissingHost { path: String },

    #[error("backend bundle not found: {path}")]
    #[diagnostic(
        code(tynd::bundler::pack::missing_backend),
        url("https://kvnpetit.github.io/tynd/docs/v1.0/concepts/tyndpkg"),
        help(
            "the `backend_bundle` field must point at a Bun-bundled JS file (e.g. produced by \
             `bun build src/backend.ts`)."
        )
    )]
    MissingBackend { path: String },

    #[error("frontend directory not found: {path}")]
    #[diagnostic(
        code(tynd::bundler::pack::missing_frontend),
        url("https://kvnpetit.github.io/tynd/docs/v1.0/concepts/tyndpkg"),
        help(
            "the `frontend_dir` field must point at the built frontend's output directory (Vite's \
             `dist/`, Next-export's `out/`, ...)."
        )
    )]
    MissingFrontend { path: String },

    #[error("icon source unsupported: {path}")]
    #[diagnostic(
        code(tynd::bundler::pack::bad_icon),
        url("https://kvnpetit.github.io/tynd/docs/v1.0/guides/icons-branding"),
        help("supported sources: .svg, .png, .ico (any case).")
    )]
    BadIcon { path: String },

    #[error("icon render failed: {cause}")]
    #[diagnostic(
        code(tynd::bundler::pack::icon_render),
        url("https://kvnpetit.github.io/tynd/docs/v1.0/guides/icons-branding"),
        help(
            "the icon source could not be parsed/rasterised. For SVG, ensure the file is \
             well-formed (try opening it in a browser); for PNG, the file must be square."
        )
    )]
    IconRender { cause: String },

    #[error("path too long ({len} bytes): {path}")]
    #[diagnostic(
        code(tynd::bundler::pack::path_too_long),
        url("https://kvnpetit.github.io/tynd/docs/v1.0/concepts/tyndpkg"),
        help(
            "TYNDPKG encodes path lengths as u16 (max 65 535 bytes). Move/rename the offending \
             asset closer to the project root."
        )
    )]
    PathTooLong { len: usize, path: String },

    #[error("asset too large ({len} bytes): {path}")]
    #[diagnostic(
        code(tynd::bundler::pack::asset_too_large),
        url("https://kvnpetit.github.io/tynd/docs/v1.0/concepts/tyndpkg"),
        help(
            "TYNDPKG encodes data lengths as u32 (max ~4 GiB). Split or compress the asset before \
             packing."
        )
    )]
    AssetTooLarge { len: u64, path: String },

    #[error("invalid embedded asset path: {path}")]
    #[diagnostic(
        code(tynd::bundler::pack::invalid_asset_path),
        help(
            "asset paths must be relative and cannot contain empty, dot, parent, drive, or UNC components"
        )
    )]
    InvalidAssetPath { path: String },

    #[error("duplicate embedded asset path: {path}")]
    #[diagnostic(
        code(tynd::bundler::pack::duplicate_asset_path),
        help("rename the colliding frontend file or sidecar so every packed path is unique")
    )]
    DuplicateAssetPath { path: String },

    #[error("PE patch failed: {cause}")]
    #[diagnostic(
        code(tynd::bundler::pack::pe_patch),
        url("https://kvnpetit.github.io/tynd/docs/v1.0/concepts/tyndpkg"),
        help(
            "the host binary could not be patched. Ensure `host_binary` is a valid PE32/PE32+ \
             executable; non-Windows targets should leave `windows.embed_icon` and \
             `windows.patch_subsystem_gui` set to false."
        )
    )]
    PePatch { cause: String },

    #[error("io error on {path}: {cause}")]
    #[diagnostic(code(tynd::bundler::pack::io))]
    Io { path: String, cause: String },
}

pub(crate) fn run(cfg: &PackConfig) -> Result<()> {
    validate_paths(cfg)?;

    let (entries, ico_bytes) = collect_pack_entries(cfg)?;
    ensure_output_parent(&cfg.output)?;
    let tmp_path = stage_tmp_path(&cfg.output)?;
    let host_bytes = prepare_host_binary(cfg, ico_bytes.as_deref())?;

    write_packed(&tmp_path, &host_bytes, &entries)?;

    #[cfg(unix)]
    set_executable(&tmp_path);

    persist_packed_output(tmp_path, &cfg.output)?;
    report_packed_output(&cfg.output);
    Ok(())
}

fn collect_pack_entries(cfg: &PackConfig) -> Result<(Vec<PackEntry>, Option<Vec<u8>>)> {
    let mut entries = collect_entries(cfg)?;

    // Build ICO once — needed for both TYNDPKG embedding and PE rsrc embedding.
    let ico_bytes = if let Some(icon) = &cfg.icon {
        let bytes = build_ico_bytes(icon).map_err(|error| {
            anyhow::Error::new(PackError::IconRender {
                cause: format!("{error:#}"),
            })
        })?;
        entries.push(PackEntry::raw("icon.ico", bytes.clone()));
        Some(bytes)
    } else {
        None
    };

    Ok((entries, ico_bytes))
}

fn ensure_output_parent(output: &Path) -> Result<()> {
    // Stage to a sibling tmp file so a crash never leaves a half-written exe.
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            anyhow::Error::new(PackError::Io {
                path: parent.display().to_string(),
                cause: error.to_string(),
            })
        })?;
    }
    Ok(())
}

fn prepare_host_binary(cfg: &PackConfig, ico_bytes: Option<&[u8]>) -> Result<Vec<u8>> {
    let host_bytes = read_file_limited(&cfg.host_binary, MAX_HOST_BYTES)?;
    let is_pe = host_bytes.len() >= 2 && &host_bytes[..2] == b"MZ";
    if is_pe {
        patch_pe_host(host_bytes, cfg, ico_bytes)
    } else {
        if cfg.windows.embed_icon || cfg.windows.patch_subsystem_gui {
            debug!("non-PE host binary — skipping windows-specific patches");
        }
        Ok(host_bytes)
    }
}

fn patch_pe_host(
    mut host_bytes: Vec<u8>,
    cfg: &PackConfig,
    ico_bytes: Option<&[u8]>,
) -> Result<Vec<u8>> {
    let manifest = cfg
        .windows
        .requested_execution_level
        .map(|level| pe::application_manifest(level.as_str()));
    if cfg.windows.embed_icon || manifest.is_some() {
        // The `.rsrc` rewrite replaces the whole section, so a manifest without
        // a configured icon still carries the default icon.
        let default_ico;
        let ico = match ico_bytes {
            Some(ico) => Some(ico),
            None if manifest.is_some() => {
                default_ico = icons::default_ico_bytes()?;
                Some(default_ico.as_slice())
            },
            None => None,
        };
        if let Some(ico) = ico {
            host_bytes = embed_host_icon_and_version(host_bytes, cfg, ico, manifest.as_deref())?;
        }
    }
    if cfg.windows.patch_subsystem_gui {
        patch_host_subsystem(&mut host_bytes, &cfg.host_binary)?;
    }
    Ok(host_bytes)
}

fn embed_host_icon_and_version(
    host_bytes: Vec<u8>,
    cfg: &PackConfig,
    ico: &[u8],
    manifest: Option<&[u8]>,
) -> Result<Vec<u8>> {
    let version_block = pe::build_version_block(&cfg.app_name, &cfg.version, None);
    let host_disp = cfg.host_binary.display().to_string();
    let icon_disp = cfg
        .icon
        .as_ref()
        .map_or_else(|| "<none>".to_string(), |path| path.display().to_string());
    let ico_len = ico.len();

    pe::embed_icon_and_version(host_bytes, ico, Some(&version_block), manifest)
        .map_err(|error| {
            Report::new(PackContext::EmbedIconRsrc).attach(format!("source = {error:#}"))
        })
        .attach_with({
            let path = host_disp.clone();
            move || format!("host_binary = {path}")
        })
        .attach_with(move || format!("icon_source = {icon_disp}"))
        .attach_with(move || format!("ico_size = {ico_len}"))
        .attach_with({
            let version = cfg.version.clone();
            move || format!("version = {version}")
        })
        .map_err(|report| {
            anyhow::Error::new(PackError::PePatch {
                cause: format!("{report:?}"),
            })
        })
}

fn patch_host_subsystem(host_bytes: &mut [u8], host_binary: &Path) -> Result<()> {
    let host_disp = host_binary.display().to_string();
    pe::patch_subsystem_gui(host_bytes)
        .map_err(|error| {
            Report::new(PackContext::PatchSubsystem).attach(format!("source = {error:#}"))
        })
        .attach_with(move || format!("host_binary = {host_disp}"))
        .attach("subsystem_target = IMAGE_SUBSYSTEM_WINDOWS_GUI(2)")
        .map_err(|report| {
            anyhow::Error::new(PackError::PePatch {
                cause: format!("{report:?}"),
            })
        })
}

#[cfg(unix)]
fn set_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;

    // chmod failures here mean the packed binary won't be executable;
    // user gets "Permission denied" at runtime with no link back to the
    // build. Surface as a warning so `tynd info` / CI logs flag it.
    if let Err(error) = fs::set_permissions(path, fs::Permissions::from_mode(0o755)) {
        tracing::warn!(
            target: "tynd::bundler::pack",
            path = %path.display(),
            "chmod 0o755 failed: {error} — packed binary may not be executable",
        );
    }
}

fn persist_packed_output(tmp_path: TempPath, output: &Path) -> Result<()> {
    tmp_path.persist(output).map_err(|error| {
        anyhow::Error::new(PackError::Io {
            path: output.display().to_string(),
            cause: error.error.to_string(),
        })
    })?;
    Ok(())
}

fn report_packed_output(output: &Path) {
    let size_mb = fs::metadata(output).map_or(0.0, |metadata| metadata.len() as f64 / 1_048_576.0);
    info!(
        "packed {} ({:.1} MB)",
        crate::util::display_path(output),
        size_mb
    );
    println!("{}", output.display());
}
fn read_file_limited(path: &Path, max_bytes: u64) -> Result<Vec<u8>> {
    let file = File::open(path).map_err(|error| {
        anyhow::Error::new(PackError::Io {
            path: path.display().to_string(),
            cause: error.to_string(),
        })
    })?;
    let size = file
        .metadata()
        .map_err(|error| {
            anyhow::Error::new(PackError::Io {
                path: path.display().to_string(),
                cause: error.to_string(),
            })
        })?
        .len();
    if size > max_bytes {
        return Err(anyhow::Error::new(PackError::AssetTooLarge {
            len: size,
            path: path.display().to_string(),
        }));
    }
    let mut bytes = Vec::with_capacity(size as usize);
    file.take(max_bytes + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| {
            anyhow::Error::new(PackError::Io {
                path: path.display().to_string(),
                cause: error.to_string(),
            })
        })?;
    if bytes.len() as u64 > max_bytes {
        return Err(anyhow::Error::new(PackError::AssetTooLarge {
            len: bytes.len() as u64,
            path: path.display().to_string(),
        }));
    }
    Ok(bytes)
}

// Validation + helpers

fn validate_paths(cfg: &PackConfig) -> Result<()> {
    if !cfg.host_binary.exists() {
        return Err(anyhow::Error::new(PackError::MissingHost {
            path: cfg.host_binary.display().to_string(),
        }));
    }
    if !cfg.backend_bundle.exists() {
        return Err(anyhow::Error::new(PackError::MissingBackend {
            path: cfg.backend_bundle.display().to_string(),
        }));
    }
    if !cfg.frontend_dir.is_dir() {
        return Err(anyhow::Error::new(PackError::MissingFrontend {
            path: cfg.frontend_dir.display().to_string(),
        }));
    }
    for s in &cfg.sidecars {
        if !s.path.exists() {
            return Err(anyhow::Error::new(PackError::Io {
                path: s.path.display().to_string(),
                cause: format!("sidecar '{}' missing", s.name),
            }));
        }
    }
    Ok(())
}

fn stage_tmp_path(out: &Path) -> Result<TempPath> {
    let parent = out.parent().unwrap_or_else(|| Path::new("."));
    let output_name = out
        .file_name()
        .map_or_else(|| "output".into(), |name| name.to_string_lossy());
    TempFileBuilder::new()
        .prefix(&format!(".{output_name}.tmp-"))
        .tempfile_in(parent)
        .map(tempfile::NamedTempFile::into_temp_path)
        .map_err(|e| {
            anyhow::Error::new(PackError::Io {
                path: parent.display().to_string(),
                cause: e.to_string(),
            })
        })
}
