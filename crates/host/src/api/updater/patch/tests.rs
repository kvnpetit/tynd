//! `downloadAndPatch` argument / authorization gates, plus the corrupt-input
//! contract of the apply step (bipatch + zstd + release re-verification).
//!
//! The full download path needs a host-trusted key pinned at compile time
//! and a manifest authorization (process-global state shared with other
//! updater tests), so it is not driven end to end here. The apply step is
//! exercised with hand-built BSDIFF (bipatch v1) containers.

use std::io::Cursor;

use ed25519_dalek::{Signer as _, SigningKey};
use serde_json::json;

use super::super::KeySet;
use super::*;

/// Encode a bipatch v1 container from `(add_diff, copy_bytes, seek)` entries.
/// All lengths used here are < 128, so each varint is a single byte; `seek`
/// is zigzag-encoded as the crate expects for `i64`.
fn bipatch(entries: &[(&[u8], &[u8], i8)]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&bipatch::MAGIC.to_le_bytes());
    out.extend_from_slice(&bipatch::VERSION.to_le_bytes());
    for (add, copy, seek) in entries {
        assert!(add.len() < 128 && copy.len() < 128);
        out.push(add.len() as u8);
        out.extend_from_slice(add);
        out.push(copy.len() as u8);
        out.extend_from_slice(copy);
        let zigzag = ((i64::from(*seek) << 1) ^ (i64::from(*seek) >> 63)) as u8;
        out.push(zigzag);
    }
    out
}

/// The apply step of `download_and_patch`, verbatim in shape.
fn apply(patch: &[u8], old: &[u8]) -> Result<Vec<u8>, String> {
    let mut reconstructed = Vec::new();
    let mut reader = bipatch::Reader::new(Cursor::new(patch), Cursor::new(old))
        .map_err(|e| format!("bipatch header: {e}"))?;
    reader
        .read_to_end(&mut reconstructed)
        .map_err(|e| format!("bipatch apply: {e}"))?;
    Ok(reconstructed)
}

const OLD: &[u8] = b"hello world";

/// `[0; 5]` add-diff keeps "hello", then copies " tynd".
fn good_patch() -> Vec<u8> {
    bipatch(&[(&[0, 0, 0, 0, 0], b" tynd", 0)])
}

#[test]
fn missing_arguments_are_rejected_before_any_io() {
    let e = download_and_patch(&json!({})).unwrap_err();
    assert_eq!(e, "updater.downloadAndPatch: missing 'url'");
    let e = download_and_patch(&json!({ "url": "https://x/p.bsdiff" })).unwrap_err();
    assert!(e.contains("missing 'signature'"), "{e}");
    let e =
        download_and_patch(&json!({ "url": "https://x/p.bsdiff", "signature": "s" })).unwrap_err();
    assert!(e.contains("missing 'releaseSignature'"), "{e}");
    let e = download_and_patch(&json!({ "url": 1, "signature": "s", "releaseSignature": "r" }))
        .unwrap_err();
    assert_eq!(
        e, "updater.downloadAndPatch: missing 'url'",
        "non-string url"
    );
}

#[test]
fn download_not_authorized_by_a_manifest_is_refused() {
    // A URL no `check` ever authorized: refused before compression parsing,
    // signature decoding or any network access (port 1 is never contacted).
    let e = download_and_patch(&json!({
        "url": "https://127.0.0.1:1/tynd-patch-test-never-authorized.bsdiff.zst",
        "signature": "not-even-base64",
        "releaseSignature": "also-bad",
        "compression": "bogus",
    }))
    .unwrap_err();
    assert!(
        e.contains("not authorized by the latest verified manifest"),
        "{e}"
    );
}

#[test]
fn well_formed_patch_reconstructs_target() {
    assert_eq!(apply(&good_patch(), OLD).unwrap(), b"hello tynd");
    // Add-diff bytes are wrapping-added onto the old binary.
    let patch = bipatch(&[(&[1, 1], b"", 0)]);
    assert_eq!(apply(&patch, b"ab").unwrap(), b"bc");
}

#[test]
fn wrong_magic_or_version_fails_at_header() {
    let mut bad_magic = good_patch();
    bad_magic[0] ^= 0xff;
    assert!(apply(&bad_magic, OLD)
        .unwrap_err()
        .starts_with("bipatch header"));

    let mut bad_version = good_patch();
    bad_version[4] ^= 0xff;
    assert!(apply(&bad_version, OLD)
        .unwrap_err()
        .starts_with("bipatch header"));

    for short in [&b""[..], &b"\xdf\xb1"[..], &good_patch()[..6]] {
        assert!(apply(short, OLD).unwrap_err().starts_with("bipatch header"));
    }
}

#[test]
fn truncation_inside_an_entry_fails_to_apply() {
    let patch = good_patch();
    // Cut inside the copy payload and right before the trailing seek varint.
    for cut in [patch.len() - 3, patch.len() - 1] {
        let e = apply(&patch[..cut], OLD).unwrap_err();
        assert!(e.starts_with("bipatch apply"), "cut {cut}: {e}");
    }
}

#[test]
fn add_past_end_of_old_binary_fails_to_apply() {
    let patch = bipatch(&[(&[0; 20], b"", 0)]);
    assert!(apply(&patch, OLD).unwrap_err().starts_with("bipatch apply"));
}

#[test]
fn truncation_at_entry_boundary_is_only_caught_by_release_signature() {
    // Dropping a whole trailing entry yields a *valid but shorter* output:
    // bipatch cannot tell. The release signature re-check is what refuses it.
    let full = bipatch(&[(&[0; 5], b" tynd", 0), (&[], b"!", 0)]);
    let header_and_first = good_patch().len();
    let expected = apply(&full, OLD).unwrap();
    assert_eq!(expected, b"hello tynd!");
    let truncated = apply(&full[..header_and_first], OLD).unwrap();
    assert_eq!(truncated, b"hello tynd");

    let signer = SigningKey::from_bytes(&[7; 32]);
    let keys = KeySet(vec![signer.verifying_key()]);
    let release_signature = signer.sign(&expected);
    assert!(keys.verify(&expected, &release_signature).is_ok());
    let e = keys.verify(&truncated, &release_signature).unwrap_err();
    assert!(e.contains("does not match"), "{e}");
}

#[test]
fn zstd_wrapped_patch_round_trips_and_corruption_is_detected() {
    let compressed = zstd::encode_all(&good_patch()[..], 3).unwrap();
    let decoded = zstd::decode_all(&compressed[..]).unwrap();
    assert_eq!(apply(&decoded, OLD).unwrap(), b"hello tynd");

    // Garbage and truncated frames fail to decode instead of yielding bytes.
    assert!(zstd::decode_all(&b"definitely not zstd"[..]).is_err());
    assert!(zstd::decode_all(&compressed[..compressed.len() - 2]).is_err());
}

#[test]
fn patch_signature_rejects_any_bit_flip() {
    let signer = SigningKey::from_bytes(&[9; 32]);
    let keys = KeySet(vec![signer.verifying_key()]);
    let patch = good_patch();
    let signature = signer.sign(&patch);
    assert!(keys.verify(&patch, &signature).is_ok());
    for i in 0..patch.len() {
        let mut tampered = patch.clone();
        tampered[i] ^= 0x01;
        assert!(
            keys.verify(&tampered, &signature).is_err(),
            "flip at byte {i}"
        );
    }
    let other = SigningKey::from_bytes(&[10; 32]).sign(&patch);
    assert!(
        keys.verify(&patch, &other).is_err(),
        "signature from an untrusted key"
    );
}
