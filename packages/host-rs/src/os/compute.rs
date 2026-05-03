use base64::{engine::general_purpose::STANDARD, Engine};
use rand::RngCore;
use serde_json::Value;
use sha2::{Digest, Sha256, Sha384, Sha512};

use crate::ipc_error::IpcError;

pub fn dispatch(method: &str, args: &Value) -> Result<Value, String> {
    match method {
        "randomBytes" => random_bytes(args),
        // hash / compress / decompress route through the `tynd-bin://`
        // custom protocol (see host-rs/src/scheme_bin.rs) — the raw-bytes
        // variants live below and avoid base64 for multi-MB payloads.
        _ => Err(IpcError::unknown_method("compute", method).into()),
    }
}

fn random_bytes(args: &Value) -> Result<Value, String> {
    let n = args
        .get("n")
        .and_then(Value::as_u64)
        .ok_or_else(|| String::from(IpcError::missing_arg("compute", "randomBytes", "n")))?;
    let buf = random_bytes_raw(n as usize)?;
    Ok(Value::String(STANDARD.encode(&buf)))
}

/// Fill `n` bytes from the OS CSPRNG. Used by both the frontend dispatch
/// path and the lite runtime's `crypto.getRandomValues` polyfill.
pub fn random_bytes_raw(n: usize) -> Result<Vec<u8>, String> {
    if n == 0 || n > 1_048_576 {
        return Err(IpcError::new(
            "compute",
            "randomBytes",
            format!("'n' must be 1..=1_048_576 (got {n})"),
        )
        .into());
    }
    let mut buf = vec![0u8; n];
    rand::rngs::OsRng.fill_bytes(&mut buf);
    Ok(buf)
}

/// Generate a fresh UUIDv7 (RFC 9562) as the canonical hyphenated lowercase
/// string. v7 IDs encode a millisecond Unix timestamp in their high bits, so
/// sorting them lexicographically also sorts them by creation time — handy
/// for IPC trace debugging and log correlation. Used by the lite QuickJS
/// runtime to back `crypto.randomUUID()` (the v4 fallback is in `crypto.js`).
#[inline]
pub fn uuid_v7_string() -> String {
    uuid::Uuid::now_v7().to_string()
}

/// Hash raw bytes with `algo` (blake3 / sha256 / sha512) and encode the digest
/// with `encoding` (hex / base64). Called by the `tynd-bin://` scheme handler.
pub fn hash_raw(data: &[u8], algo: &str, encoding: &str) -> Result<String, String> {
    let digest: Vec<u8> = match algo {
        "blake3" => blake3::hash(data).as_bytes().to_vec(),
        "sha256" => Sha256::digest(data).to_vec(),
        "sha384" => Sha384::digest(data).to_vec(),
        "sha512" => Sha512::digest(data).to_vec(),
        _ => {
            return Err(IpcError::new(
                "compute",
                "hash",
                format!("unsupported algo '{algo}'"),
            )
            .into())
        },
    };
    match encoding {
        "hex" => Ok(hex_encode(&digest)),
        "base64" => Ok(STANDARD.encode(&digest)),
        _ => Err(
            IpcError::new("compute", "hash", format!("unsupported encoding '{encoding}'")).into(),
        ),
    }
}

/// Hex-encode `bytes` as lowercase ASCII. Backed by `faster-hex`'s SIMD
/// path (SSE4.1/AVX2 on x86_64, NEON on aarch64, scalar fallback elsewhere)
/// — 5-10× faster than the hand-rolled byte loop on modern hardware while
/// producing the same lowercase output as the previous implementation.
fn hex_encode(bytes: &[u8]) -> String {
    let mut out = vec![0u8; bytes.len() * 2];
    // `hex_encode` writes ASCII into `out` and only fails when the
    // destination is the wrong length, which we control above. The
    // `unwrap` is therefore safe and never trips at runtime.
    faster_hex::hex_encode(bytes, &mut out).expect("faster_hex output buffer sized correctly");
    // SAFETY: `faster_hex::hex_encode` writes only ASCII hex digits.
    unsafe { String::from_utf8_unchecked(out) }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn hash_known_sha256_vector() {
        let out = hash_raw(b"abc", "sha256", "hex").unwrap();
        assert_eq!(
            out,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn hash_known_sha384_vector() {
        let out = hash_raw(b"abc", "sha384", "hex").unwrap();
        assert_eq!(
            out,
            "cb00753f45a35e8bb5a03d699ac65007272c32ab0eded1631a8b605a43ff5bed\
             8086072ba1e7cc2358baeca134c825a7"
        );
    }

    #[test]
    fn hash_blake3_deterministic() {
        assert_eq!(
            hash_raw(b"hello", "blake3", "hex").unwrap(),
            hash_raw(b"hello", "blake3", "hex").unwrap()
        );
    }

    #[test]
    fn hash_rejects_unknown_algo() {
        assert!(hash_raw(b"x", "md5", "hex").is_err());
    }

    #[test]
    fn random_bytes_respects_length_and_is_distinct() {
        let a = random_bytes(&json!({ "n": 32 })).unwrap();
        let b = random_bytes(&json!({ "n": 32 })).unwrap();
        let a_bytes = STANDARD.decode(a.as_str().unwrap()).unwrap();
        let b_bytes = STANDARD.decode(b.as_str().unwrap()).unwrap();
        assert_eq!(a_bytes.len(), 32);
        assert_eq!(b_bytes.len(), 32);
        // Two separate calls must not collide (probability ~ 2^-256).
        assert_ne!(a_bytes, b_bytes);
    }

    #[test]
    fn random_bytes_rejects_out_of_range() {
        assert!(random_bytes(&json!({ "n": 0 })).is_err());
        assert!(random_bytes(&json!({ "n": 2_000_000 })).is_err());
        assert!(random_bytes(&json!({})).is_err());
    }
}
