use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use rand::RngCore;
use serde_json::Value;
use sha2::{Digest, Sha256, Sha512};

pub fn dispatch(method: &str, args: &Value) -> Result<Value, String> {
    match method {
        "randomBytes" => random_bytes(args),
        // hash / compress / decompress route through the `tynd-bin://`
        // custom protocol (see host-rs/src/scheme_bin.rs) — the raw-bytes
        // variants live below and avoid base64 for multi-MB payloads.
        _ => Err(format!("compute.{method}: unknown method")),
    }
}

fn random_bytes(args: &Value) -> Result<Value, String> {
    let n = args
        .get("n")
        .and_then(Value::as_u64)
        .ok_or_else(|| "compute.randomBytes: missing 'n'".to_string())?;
    if n == 0 || n > 1_048_576 {
        return Err(format!(
            "compute.randomBytes: 'n' must be 1..=1_048_576 (got {n})"
        ));
    }
    let mut buf = vec![0u8; n as usize];
    rand::rngs::OsRng.fill_bytes(&mut buf);
    Ok(Value::String(STANDARD.encode(&buf)))
}

/// Hash raw bytes with `algo` (blake3 / sha256 / sha512) and encode the digest
/// with `encoding` (hex / base64). Called by the `tynd-bin://` scheme handler.
pub fn hash_raw(data: &[u8], algo: &str, encoding: &str) -> Result<String, String> {
    let digest: Vec<u8> = match algo {
        "blake3" => blake3::hash(data).as_bytes().to_vec(),
        "sha256" => Sha256::digest(data).to_vec(),
        "sha512" => Sha512::digest(data).to_vec(),
        _ => return Err(format!("compute.hash: unsupported algo '{algo}'")),
    };
    match encoding {
        "hex" => Ok(hex_encode(&digest)),
        "base64" => Ok(STANDARD.encode(&digest)),
        _ => Err(format!("compute.hash: unsupported encoding '{encoding}'")),
    }
}

pub fn compress_raw(data: &[u8], algo: &str, level: i32) -> Result<Vec<u8>, String> {
    match algo {
        "zstd" => {
            zstd::stream::encode_all(data, level).map_err(|e| format!("compute.compress: {e}"))
        },
        _ => Err(format!("compute.compress: unsupported algo '{algo}'")),
    }
}

pub fn decompress_raw(data: &[u8], algo: &str) -> Result<Vec<u8>, String> {
    match algo {
        "zstd" => zstd::stream::decode_all(data).map_err(|e| format!("compute.decompress: {e}")),
        _ => Err(format!("compute.decompress: unsupported algo '{algo}'")),
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0xf) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn hash_known_sha256_vector() {
        let out = hash_raw(b"abc", "sha256", "hex").unwrap();
        assert_eq!(
            out,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
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

    #[test]
    fn zstd_roundtrip_restores_bytes() {
        let original: Vec<u8> = (0..=255u8).collect();
        let compressed = compress_raw(&original, "zstd", 3).unwrap();
        let decompressed = decompress_raw(&compressed, "zstd").unwrap();
        assert_eq!(decompressed, original);
    }
}
