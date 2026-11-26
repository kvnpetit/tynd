use std::{fs, io::Read};

use bytes::{Bytes, BytesMut};
use serde_json::{json, Value};

use super::install::register_verified_artifact;
use super::{
    decode_signature, get_with_redirects, hashes_match, http_agent, require_authorized_download,
    write_staged_blob, KeySet, PatchCompression, Progress,
};

/// Download a BSDIFF patch, verify its Ed25519 signature + BLAKE3,
/// apply it on top of the currently-running binary to reconstruct the
/// new release, then re-verify the *reconstructed* binary against the
/// release-level signature from the manifest. Output shape is
/// identical to `download_and_verify` (opaque handle + size) so the JS layer
/// can hand the result straight to `install` without branching.
///
/// Required args:
/// - `url`            : patch download URL (`.bsdiff` or `.bsdiff.zst`)
/// - `signature`      : base64 Ed25519 over the *patch bytes*
/// - `releaseSignature`: base64 Ed25519 over the *reconstructed binary* (this is the per-platform
///   `signature` field from the manifest)
/// - `compression`    : "zstd" | "none" (default: detect from URL suffix)
///
/// Optional:
/// - `expectedBlake3` : hex BLAKE3 of the patch bytes (defense in depth)
/// - `progressId` — same semantics as the full path.
///
/// Verifying keys, proxy and headers are never read from `args`: they come from the host-trusted
/// updater configuration (any trusted key may verify a signature, for key rotation).
pub(super) fn download_and_patch(args: &Value) -> Result<Value, String> {
    let url = args
        .get("url")
        .and_then(Value::as_str)
        .ok_or("updater.downloadAndPatch: missing 'url'")?;
    let patch_sig_b64 = args
        .get("signature")
        .and_then(Value::as_str)
        .ok_or("updater.downloadAndPatch: missing 'signature' (Ed25519 over patch bytes)")?;
    let release_sig_b64 = args.get("releaseSignature").and_then(Value::as_str).ok_or(
        "updater.downloadAndPatch: missing 'releaseSignature' (Ed25519 over the reconstructed new \
         binary; required so a malicious patch can't decompose to an unsigned binary)",
    )?;
    require_authorized_download(url, patch_sig_b64, "binary", Some(release_sig_b64))?;
    let progress_id = args
        .get("progressId")
        .and_then(Value::as_str)
        .map(String::from);
    let expected_patch_blake3 = args.get("expectedBlake3").and_then(Value::as_str);
    // Default to `zstd` when the URL ends with `.zst` — keeps the
    // manifest format flexible without forcing the field on every entry.
    // Case-insensitive so `.ZST` from manifests authored on case-preserving
    // filesystems (Windows) doesn't fall through to `none` and leave a
    // compressed payload undecoded.
    let compression = PatchCompression::from_manifest_or_url(
        args.get("compression").and_then(Value::as_str),
        url,
    )
    .map_err(|e| format!("updater.downloadAndPatch: {e}"))?;

    let patch_signature = decode_signature(patch_sig_b64)?;
    let release_signature = decode_signature(release_sig_b64)?;
    let keys = KeySet::from_trusted("updater.downloadAndPatch")?;

    let (patch_bytes, mut emitter) = download_patch_bytes(url, progress_id)?;

    // Verify patch signature + (optional) BLAKE3.
    keys.verify(&patch_bytes, &patch_signature)
        .map_err(|e| format!("updater.downloadAndPatch: patch signature check failed: {e}"))?;

    if let Some(expected) = expected_patch_blake3 {
        let actual = blake3::hash(&patch_bytes).to_hex().to_string();
        if !hashes_match(expected, &actual) {
            return Err(format!(
                "updater.downloadAndPatch: patch BLAKE3 mismatch — expected {expected}, got \
                 {actual}; refusing to apply"
            ));
        }
    }

    emitter.emit_phase("verified-patch");

    let reconstructed = reconstruct_binary(&patch_bytes, compression, &mut emitter)?;

    // Re-verify the RECONSTRUCTED binary against the release signature.
    // This is the load-bearing check: it makes sure the patch produced
    // exactly the bytes the release signer covered. Without it, an
    // attacker who could MITM the patch could craft any payload they
    // want — the patch sig only authenticates the patch itself, not what
    // it computes to.
    keys.verify(&reconstructed, &release_signature)
        .map_err(|e| {
            format!(
                "updater.downloadAndPatch: reconstructed binary failed release signature \
                 verification: {e} (patch may be authentic but doesn't reproduce the signed \
                 release; refusing to install)"
            )
        })?;

    let verified_hash = blake3::hash(&reconstructed);

    // Stage to temp dir for the `install` step.
    let tmp = write_staged_blob(&reconstructed)
        .map_err(|e| format!("updater.downloadAndPatch: stage reconstructed payload: {e}"))?;
    let handle = register_verified_artifact(&tmp, verified_hash.to_hex().to_string(), "binary")
        .map_err(|error| {
            let _ = fs::remove_file(&tmp);
            format!("updater.downloadAndPatch: {error}")
        })?;

    emitter.emit_phase("verified");

    Ok(json!({
        "handle": handle,
        "size": reconstructed.len(),
        // Echoed back so frontends can show the actual savings ratio
        // (compared to advertising it from the manifest only).
        "patchSize": patch_bytes.len(),
    }))
}

/// Decompress the verified patch (if needed) and apply it on top of the
/// currently-running binary.
fn reconstruct_binary(
    patch_bytes: &[u8],
    compression: PatchCompression,
    emitter: &mut Progress,
) -> Result<Vec<u8>, String> {
    // `bipatch` reads via a `Cursor<&[u8]>` either way; for the `"none"`
    // branch this is one memcpy out of the refcounted download buffer
    // before the long-running patch apply.
    let patch_payload: Vec<u8> = match compression {
        PatchCompression::Zstd => zstd::decode_all(patch_bytes)
            .map_err(|e| format!("updater.downloadAndPatch: zstd decode: {e}"))?,
        PatchCompression::None => patch_bytes.to_vec(),
    };

    let current_exe = std::env::current_exe()
        .map_err(|e| format!("updater.downloadAndPatch: current_exe: {e}"))?;
    let old_bytes = fs::read(&current_exe).map_err(|e| format!("updater.downloadAndPatch: {e}"))?;

    emitter.emit_phase("applying");

    let mut reconstructed: Vec<u8> = Vec::with_capacity(old_bytes.len());
    let mut bipatch_reader = bipatch::Reader::new(
        std::io::Cursor::new(&patch_payload),
        std::io::Cursor::new(&old_bytes),
    )
    .map_err(|e| format!("updater.downloadAndPatch: bipatch header: {e}"))?;
    bipatch_reader
        .read_to_end(&mut reconstructed)
        .map_err(|e| format!("updater.downloadAndPatch: bipatch apply: {e}"))?;
    Ok(reconstructed)
}

fn download_patch_bytes(
    url: &str,
    progress_id: Option<String>,
) -> Result<(Bytes, Progress), String> {
    let agent = http_agent()?;
    let mut response = get_with_redirects(&agent, url, "updater.downloadAndPatch")?;
    let total = response
        .headers()
        .get("Content-Length")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok());

    let mut emitter = Progress::new(progress_id, "download", total);
    let mut reader = response.body_mut().as_reader();
    let mut buffer = vec![0_u8; 64 * 1024];
    const MAX_PATCH_BYTES: usize = 256 * 1024 * 1024;
    let mut patch = BytesMut::with_capacity(total.unwrap_or(0).min(8 * 1024 * 1024) as usize);
    loop {
        let read = reader
            .read(&mut buffer)
            .map_err(|error| format!("updater.downloadAndPatch: read: {error}"))?;
        if read == 0 {
            emitter.advance(0, true);
            break;
        }
        patch.extend_from_slice(&buffer[..read]);
        if patch.len() > MAX_PATCH_BYTES {
            return Err(format!(
                "updater.downloadAndPatch: patch exceeds {MAX_PATCH_BYTES} byte safety cap; \
                 refusing"
            ));
        }
        emitter.advance(read, false);
    }
    Ok((patch.freeze(), emitter))
}

#[cfg(test)]
mod tests;
