use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use rand::RngCore;
use serde_json::{json, Value};
use sha2::{Digest, Sha256, Sha512};

pub fn dispatch(method: &str, args: &Value) -> Result<Value, String> {
    match method {
        "hash" => hash(args),
        "compress" => compress(args),
        "decompress" => decompress(args),
        "randomBytes" => random_bytes(args),
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

fn data_arg(args: &Value) -> Result<Vec<u8>, String> {
    let b64 = args
        .get("data")
        .and_then(Value::as_str)
        .ok_or_else(|| "compute: missing base64 'data'".to_string())?;
    STANDARD
        .decode(b64)
        .map_err(|e| format!("compute: invalid base64: {e}"))
}

fn hash(args: &Value) -> Result<Value, String> {
    let data = data_arg(args)?;
    let algo = args.get("algo").and_then(Value::as_str).unwrap_or("blake3");
    let encoding = args
        .get("encoding")
        .and_then(Value::as_str)
        .unwrap_or("hex");

    let digest: Vec<u8> = match algo {
        "blake3" => blake3::hash(&data).as_bytes().to_vec(),
        "sha256" => Sha256::digest(&data).to_vec(),
        "sha512" => Sha512::digest(&data).to_vec(),
        _ => return Err(format!("compute.hash: unsupported algo '{algo}'")),
    };

    let out = match encoding {
        "hex" => hex_encode(&digest),
        "base64" => STANDARD.encode(&digest),
        _ => return Err(format!("compute.hash: unsupported encoding '{encoding}'")),
    };
    Ok(Value::String(out))
}

fn compress(args: &Value) -> Result<Value, String> {
    let data = data_arg(args)?;
    let algo = args.get("algo").and_then(Value::as_str).unwrap_or("zstd");
    let level = args.get("level").and_then(Value::as_i64).unwrap_or(3) as i32;

    let out = match algo {
        "zstd" => zstd::stream::encode_all(&data[..], level)
            .map_err(|e| format!("compute.compress: {e}"))?,
        _ => return Err(format!("compute.compress: unsupported algo '{algo}'")),
    };
    Ok(Value::String(STANDARD.encode(&out)))
}

fn decompress(args: &Value) -> Result<Value, String> {
    let data = data_arg(args)?;
    let algo = args.get("algo").and_then(Value::as_str).unwrap_or("zstd");

    let out = match algo {
        "zstd" => {
            zstd::stream::decode_all(&data[..]).map_err(|e| format!("compute.decompress: {e}"))?
        },
        _ => return Err(format!("compute.decompress: unsupported algo '{algo}'")),
    };
    Ok(json!({ "data": STANDARD.encode(&out), "bytes": out.len() }))
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

    fn args(data: &[u8], algo: &str) -> Value {
        json!({ "data": STANDARD.encode(data), "algo": algo })
    }

    #[test]
    fn hash_known_sha256_vector() {
        let out = hash(&args(b"abc", "sha256")).unwrap();
        assert_eq!(
            out.as_str().unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn hash_blake3_deterministic() {
        let a = hash(&args(b"hello", "blake3")).unwrap();
        let b = hash(&args(b"hello", "blake3")).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn hash_rejects_unknown_algo() {
        assert!(hash(&args(b"x", "md5")).is_err());
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
        let original = (0..=255u8).collect::<Vec<_>>();
        let compressed = compress(&args(&original, "zstd")).unwrap();
        let decompressed = decompress(&json!({
            "data": compressed.as_str().unwrap(),
            "algo": "zstd"
        }))
        .unwrap();
        let decoded = STANDARD
            .decode(decompressed["data"].as_str().unwrap())
            .unwrap();
        assert_eq!(decoded, original);
    }
}
