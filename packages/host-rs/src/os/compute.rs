use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use serde_json::{json, Value};
use sha2::{Digest, Sha256, Sha512};

pub fn dispatch(method: &str, args: &Value) -> Result<Value, String> {
    match method {
        "hash" => hash(args),
        "compress" => compress(args),
        "decompress" => decompress(args),
        _ => Err(format!("compute.{method}: unknown method")),
    }
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
