use std::io::Read;

use bytes::{Bytes, BytesMut};
use serde_json::{json, Value};

use super::install::register_verified_artifact;
use super::{
    decode_signature, get_with_redirects, http_agent, require_authorized_download,
    write_staged_blob, KeySet, Progress,
};

/// Hard cap on full-binary download size — protects against hostile or
/// misconfigured CDNs that could otherwise OOM the host. 2 GiB so
/// legitimate desktop binaries (Electron-class apps, GPU runtimes) stay
/// well within.
const MAX_BINARY_BYTES: u64 = 2 * 1024 * 1024 * 1024;

pub(super) fn download_and_verify(args: &Value) -> Result<Value, String> {
    let url = args
        .get("url")
        .and_then(Value::as_str)
        .ok_or("updater.downloadAndVerify: missing 'url'")?;
    let signature_b64 = args
        .get("signature")
        .and_then(Value::as_str)
        .ok_or("updater.downloadAndVerify: missing 'signature'")?;
    let progress_id = args
        .get("progressId")
        .and_then(Value::as_str)
        .map(String::from);
    let format = args
        .get("format")
        .and_then(Value::as_str)
        .unwrap_or("binary");
    if !matches!(format, "binary" | "app-archive") {
        return Err(format!(
            "updater.downloadAndVerify: unsupported artifact format '{format}'"
        ));
    }
    require_authorized_download(url, signature_b64, format, None)?;

    let signature = decode_signature(signature_b64)?;
    let keys = KeySet::from_trusted("updater.downloadAndVerify")?;

    let (bytes_accum, mut emitter) = download_capped(url, progress_id)?;

    // The payload is an immutable, refcounted `Bytes`, read three times
    // below (Ed25519 verify, BLAKE3 hash, on-disk write).
    keys.verify(&bytes_accum, &signature)
        .map_err(|e| format!("updater.downloadAndVerify: signature check failed: {e}"))?;

    // BLAKE3 of the *verified* bytes. `install` will re-hash the file
    // it's about to swap in and refuse the swap on mismatch — closes
    // the TOCTOU between this function returning and `install` calling
    // `self_replace`.
    let verified_hash = blake3::hash(&bytes_accum);

    let tmp = write_staged_blob(&bytes_accum)
        .map_err(|e| format!("updater.downloadAndVerify: stage verified payload: {e}"))?;
    let handle = register_verified_artifact(&tmp, verified_hash.to_hex().to_string(), format)
        .map_err(|error| {
            let _ = std::fs::remove_file(&tmp);
            format!("updater.downloadAndVerify: {error}")
        })?;

    emitter.emit_phase("verified");

    Ok(json!({
        "handle": handle,
        "size": bytes_accum.len(),
    }))
}

/// Stream `url` into memory under [`MAX_BINARY_BYTES`], reporting progress.
fn download_capped(url: &str, progress_id: Option<String>) -> Result<(Bytes, Progress), String> {
    let agent = http_agent()?;
    let mut resp = get_with_redirects(&agent, url, "updater.downloadAndVerify")?;
    let total = resp
        .headers()
        .get("Content-Length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok());

    // Reject up-front if the server advertises more than the cap — saves
    // us from streaming gigabytes only to error at the end.
    if let Some(advertised) = total {
        if advertised > MAX_BINARY_BYTES {
            return Err(format!(
                "updater.downloadAndVerify: refusing {advertised}-byte download (Content-Length exceeds {MAX_BINARY_BYTES}-byte cap)"
            ));
        }
    }

    let mut emitter = Progress::new(progress_id, "download", total);
    // Bound the read with `take(MAX_BINARY_BYTES + 1)` so a server that
    // keeps streaming past the cap (no Content-Length, lying CDN) still
    // hits a hard limit. Cumulative check below catches the +1 byte.
    let mut reader = resp.body_mut().as_reader().take(MAX_BINARY_BYTES + 1);
    let mut buf = vec![0u8; 64 * 1024];
    // Cap the pre-allocation at 8 MiB so a hostile/incorrect Content-Length
    // can't make us reserve gigabytes up-front. `BytesMut` grows
    // amortized-O(1) past the initial capacity.
    let mut accum = BytesMut::with_capacity(total.unwrap_or(0).min(8 * 1024 * 1024) as usize);
    let mut total_read: u64 = 0;
    loop {
        let n = reader
            .read(&mut buf)
            .map_err(|e| format!("updater.downloadAndVerify: read: {e}"))?;
        if n == 0 {
            emitter.advance(0, true);
            break;
        }
        total_read = total_read.saturating_add(n as u64);
        if total_read > MAX_BINARY_BYTES {
            return Err(format!(
                "updater.downloadAndVerify: download exceeded {MAX_BINARY_BYTES}-byte cap"
            ));
        }
        accum.extend_from_slice(&buf[..n]);
        emitter.advance(n, false);
    }
    Ok((accum.freeze(), emitter))
}
