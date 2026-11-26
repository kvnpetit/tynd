//! BSDIFF patch generator. tynd-host consumes via `bipatch::Reader`.
//!
//! `old`/`new` are applied byte-for-byte: the patch is applied to `current_exe`
//! as shipped. When the published artifacts are code-signed (Authenticode
//! certificate table / macOS signature appended after the pack), pass the
//! signed bytes of both versions. The signature blob is just trailing data to
//! the differ, so a patch between signed artifacts is valid but bigger than
//! one between unsigned binaries (the signatures differ). Mixing a signed and
//! an unsigned input yields a patch that only applies to the exact bytes it
//! was built from.

use std::{
    fmt,
    path::{Path, PathBuf},
};

use error_stack::{Report, ResultExt};
use miette::Diagnostic;
use thiserror::Error;
use tracing::info;
use tynd_update_core::PatchCompression;

/// `error-stack` context markers; rendered into `PatchError`'s `source_msg`
/// at the diagnostic boundary so users see input sizes / paths instead of
/// just the bare `io::Error::Display`.
#[derive(Debug)]
pub(crate) enum PatchContext {
    ReadOldBinary,
    ReadNewBinary,
    BidiffSimpleDiff,
    ZstdEncode,
    WriteOutput,
}

impl fmt::Display for PatchContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ReadOldBinary => f.write_str("read previous shipped binary"),
            Self::ReadNewBinary => f.write_str("read new binary"),
            Self::BidiffSimpleDiff => f.write_str("compute bidiff between old and new"),
            Self::ZstdEncode => f.write_str("zstd-encode bidiff payload"),
            Self::WriteOutput => f.write_str("write patch file"),
        }
    }
}

impl std::error::Error for PatchContext {}

#[derive(Debug, Clone, Error, Diagnostic)]
pub(crate) enum PatchError {
    #[error("failed to read old binary: {path}")]
    #[diagnostic(
        code(tynd::bundler::patch::read_old),
        help(
            "the `--old` path must point to the previously-released binary, exactly as shipped (signed if you sign). If you \
             only have the installed `.app`/`.exe`, extract the inner Mach-O / PE first."
        )
    )]
    ReadOld { path: String, source_msg: String },

    #[error("failed to read new binary: {path}")]
    #[diagnostic(
        code(tynd::bundler::patch::read_new),
        help(
            "the `--new` path must be the binary exactly as it will ship (signed if you sign), produced by the same \
             `tynd build` step that drives the rest of the release."
        )
    )]
    ReadNew { path: String, source_msg: String },

    #[error("bidiff failed: {msg}")]
    #[diagnostic(
        code(tynd::bundler::patch::diff),
        help(
            "bidiff scan/sort failed. The most common cause is an oversized input — patches are \
             designed for binaries up to ~1 GB; beyond that, prefer full downloads."
        )
    )]
    Diff { msg: String },

    #[error("zstd compression failed: {msg}")]
    #[diagnostic(
        code(tynd::bundler::patch::compress),
        help(
            "zstd `encode_all` failed. Retry with `--no-compress` to confirm the diff itself \
             succeeded; if so, file an issue with the input sizes."
        )
    )]
    Compress { msg: String },

    #[error("failed to write patch: {path}")]
    #[diagnostic(
        code(tynd::bundler::patch::write),
        help(
            "the `--output` path must be writable. Create parent directories ahead of time — the \
             bundler does not auto-mkdir."
        )
    )]
    Write { path: String, source_msg: String },
}

#[derive(Debug, Clone)]
pub(crate) struct PatchArgs {
    pub old: PathBuf,
    pub new: PathBuf,
    pub output: PathBuf,
    /// Wrap in single-frame zstd at level 19 (saves ~10-15% over level 9 with
    /// negligible client-side decompression cost).
    pub compress: bool,
}

pub(crate) fn run(args: &PatchArgs) -> miette::Result<()> {
    let old_bytes = read_old_binary(&args.old)?;
    let new_bytes = read_new_binary(&args.new)?;

    info!(
        "diffing old={} bytes, new={} bytes",
        old_bytes.len(),
        new_bytes.len()
    );

    let patch = build_patch(&old_bytes, &new_bytes)?;
    let compression = compression_for(args.compress);
    let payload = compress_patch(&patch, compression)?;
    write_patch(&args.output, &payload)?;
    report_patch(&patch, &payload, new_bytes.len(), compression);

    Ok(())
}

fn compression_for(compress: bool) -> PatchCompression {
    if compress {
        PatchCompression::Zstd
    } else {
        PatchCompression::None
    }
}

fn read_old_binary(path: &Path) -> miette::Result<Vec<u8>> {
    let display = path.display().to_string();
    std::fs::read(path)
        .map_err(Report::from)
        .change_context(PatchContext::ReadOldBinary)
        .attach_with({
            let path = display.clone();
            move || format!("old_path = {path}")
        })
        .map_err(|report| {
            miette::Report::new(PatchError::ReadOld {
                path: display,
                source_msg: format!("{report:?}"),
            })
        })
}

fn read_new_binary(path: &Path) -> miette::Result<Vec<u8>> {
    let display = path.display().to_string();
    std::fs::read(path)
        .map_err(Report::from)
        .change_context(PatchContext::ReadNewBinary)
        .attach_with({
            let path = display.clone();
            move || format!("new_path = {path}")
        })
        .map_err(|report| {
            miette::Report::new(PatchError::ReadNew {
                path: display,
                source_msg: format!("{report:?}"),
            })
        })
}

fn build_patch(old_bytes: &[u8], new_bytes: &[u8]) -> miette::Result<Vec<u8>> {
    let mut patch = Vec::with_capacity(new_bytes.len() / 4);
    bidiff::simple_diff(old_bytes, new_bytes, &mut patch)
        .map_err(Report::from)
        .change_context(PatchContext::BidiffSimpleDiff)
        .attach_with({
            let size = old_bytes.len();
            move || format!("old_size = {size}")
        })
        .attach_with({
            let size = new_bytes.len();
            move || format!("new_size = {size}")
        })
        .map_err(|report| {
            miette::Report::new(PatchError::Diff {
                msg: format!("{report:?}"),
            })
        })?;
    Ok(patch)
}

fn compress_patch(patch: &[u8], compression: PatchCompression) -> miette::Result<Vec<u8>> {
    if compression == PatchCompression::Zstd {
        zstd::encode_all(patch, 19)
            .map_err(Report::from)
            .change_context(PatchContext::ZstdEncode)
            .attach_with({
                let size = patch.len();
                move || format!("raw_patch_size = {size}")
            })
            .attach("zstd_level = 19")
            .map_err(|report| {
                miette::Report::new(PatchError::Compress {
                    msg: format!("{report:?}"),
                })
            })
    } else {
        Ok(patch.to_vec())
    }
}

fn write_patch(path: &Path, payload: &[u8]) -> miette::Result<()> {
    let display = path.display().to_string();
    std::fs::write(path, payload)
        .map_err(Report::from)
        .change_context(PatchContext::WriteOutput)
        .attach_with({
            let path = display.clone();
            move || format!("output_path = {path}")
        })
        .attach_with({
            let size = payload.len();
            move || format!("payload_size = {size}")
        })
        .map_err(|report| {
            miette::Report::new(PatchError::Write {
                path: display,
                source_msg: format!("{report:?}"),
            })
        })
}

fn report_patch(
    patch: &[u8],
    payload: &[u8],
    new_binary_len: usize,
    compression: PatchCompression,
) {
    let new_len = new_binary_len.max(1);
    let ratio = (payload.len() as f64 / new_len as f64) * 100.0;
    println!(
        "Patch generated: {} bytes ({:.1}% of new binary)",
        payload.len(),
        ratio
    );
    if compression == PatchCompression::Zstd {
        let raw_ratio = (patch.len() as f64 / new_len as f64) * 100.0;
        info!(
            "raw bidiff: {} bytes ({:.1}% of new), compressed: {} bytes ({:.1}%)",
            patch.len(),
            raw_ratio,
            payload.len(),
            ratio
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pseudo_random(len: usize, seed: u32) -> Vec<u8> {
        let mut state = seed;
        (0..len)
            .map(|_| {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (state >> 24) as u8
            })
            .collect()
    }

    /// Host bytes + pack trailer + appended signature-style blob.
    fn signed_style(body: &[u8], signature_seed: u32) -> Vec<u8> {
        let mut bytes = body.to_vec();
        while !bytes.len().is_multiple_of(8) {
            bytes.push(0);
        }
        bytes.extend_from_slice(&pseudo_random(2048, signature_seed));
        bytes
    }

    #[test]
    fn patches_between_signed_style_artifacts_are_built_and_compact() {
        let old_body = pseudo_random(64 * 1024, 1);
        let mut new_body = old_body.clone();
        new_body[1000..1016].copy_from_slice(&[0xAA; 16]);

        let old = signed_style(&old_body, 10);
        let new = signed_style(&new_body, 20);
        let patch = build_patch(&old, &new).expect("signed inputs diff");
        let compressed = compress_patch(&patch, PatchCompression::Zstd).expect("compress");

        assert_ne!(patch, [] as [u8; 0]);
        // The body is nearly identical: only the edit and the signature blob
        // (random, ~2 KiB) are new data.
        assert!(
            compressed.len() < new.len() / 4,
            "patch of {} bytes for a {} byte artifact",
            compressed.len(),
            new.len()
        );
    }
}
