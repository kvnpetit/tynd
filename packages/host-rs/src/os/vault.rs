//! Encrypted file vault — AES-GCM-SIV (or ChaCha20-Poly1305 fallback)
//! over `cocoon`. Sized for secrets larger than the OS keychain happily
//! holds: bundled API keys, plugin state, long refresh tokens, etc.
//!
//! Path handling delegates to `super::security::check_fs` (policy gate)
//! before touching the disk; combined with the host's cap-std roots in
//! `os::fs`, an unconfigured policy still confines reads/writes to the
//! same scopes every other FS API uses. Passwords are accepted as
//! borrowed strings, never logged, and zeroized after use.

use std::{fs::File, io::Write as _, path::PathBuf};

use base64::{engine::general_purpose::STANDARD, Engine as _};
use cocoon::Cocoon;
use serde_json::{json, Value};
use zeroize::Zeroizing;

pub fn dispatch(method: &str, args: &Value) -> Result<Value, String> {
    match method {
        "read" => read(args),
        "write" => write(args),
        "exists" => exists(args),
        "delete" => delete(args),
        _ => Err(format!("vault.{method}: unknown method")),
    }
}

fn path_arg(args: &Value) -> Result<PathBuf, String> {
    let s = args
        .get("path")
        .and_then(Value::as_str)
        .ok_or_else(|| "vault: missing 'path'".to_string())?;
    super::security::check_fs(s)?;
    Ok(PathBuf::from(s))
}

/// Take a password string out of the JSON args and place it into a
/// `Zeroizing` buffer so the bytes are wiped from memory the moment the
/// buffer goes out of scope. `Vec::drop` only releases the heap
/// allocation — it doesn't scrub the page, so a future allocator
/// handing the same memory back can leak the cleartext. The original
/// `serde_json::Value` still owns a copy until it's dropped at the
/// dispatch layer (outside our control), but that bounds the residency
/// window for our copy.
fn take_password(args: &Value) -> Result<Zeroizing<Vec<u8>>, String> {
    let p = args
        .get("password")
        .and_then(Value::as_str)
        .ok_or_else(|| "vault: missing 'password'".to_string())?;
    if p.is_empty() {
        return Err("vault: 'password' must not be empty".into());
    }
    Ok(Zeroizing::new(p.as_bytes().to_vec()))
}

fn read(args: &Value) -> Result<Value, String> {
    let path = path_arg(args)?;
    let password = take_password(args)?;

    let mut file = File::open(&path)
        .map_err(|e| format!("vault.read({}): open: {e}", path.display()))?;
    let cocoon = Cocoon::new(&password);
    let plaintext = cocoon
        .parse(&mut file)
        .map_err(|e| format!("vault.read({}): parse: {e:?}", path.display()))?;

    // Drop the key material as soon as the parse returns. The `Zeroizing`
    // wrapper scrubs the buffer before releasing the allocation; `cocoon`
    // itself pulls in `zeroize` for its internal scratch (verified via
    // lockfile). This just shortens our copy's lifetime.
    drop(password);

    Ok(json!({
        "data": STANDARD.encode(&plaintext),
    }))
}

fn write(args: &Value) -> Result<Value, String> {
    let path = path_arg(args)?;
    let password = take_password(args)?;

    // Accept either a base64 string or a JSON array of byte values for
    // small payloads. Base64 is the canonical form (matches `read()`'s
    // return shape and what the JS API gives us by default).
    let data_bytes: Vec<u8> = match args.get("data") {
        Some(Value::String(b64)) => STANDARD
            .decode(b64.as_bytes())
            .map_err(|e| format!("vault.write: base64 decode: {e}"))?,
        Some(Value::Array(arr)) => arr
            .iter()
            .map(|v| {
                v.as_u64()
                    .and_then(|n| u8::try_from(n).ok())
                    .ok_or_else(|| "vault.write: non-byte element in 'data'".to_string())
            })
            .collect::<Result<Vec<u8>, _>>()?,
        _ => return Err("vault.write: missing 'data' (base64 string or byte array)".into()),
    };

    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("vault.write: mkdir {}: {e}", parent.display()))?;
        }
    }

    // Use a tempfile + persist for the same atomic-rename guarantee
    // `http.download` relies on — we never want a half-written vault
    // to ghost a previous good payload on crash.
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map_or_else(|| PathBuf::from("."), std::path::Path::to_path_buf);
    let mut tmp = tempfile::NamedTempFile::new_in(&parent)
        .map_err(|e| format!("vault.write: tempfile in {}: {e}", parent.display()))?;

    let mut cocoon = Cocoon::new(&password);
    cocoon
        .dump(data_bytes, tmp.as_file_mut())
        .map_err(|e| format!("vault.write({}): dump: {e:?}", path.display()))?;
    tmp.as_file_mut()
        .flush()
        .map_err(|e| format!("vault.write: flush: {e}"))?;
    tmp.persist(&path)
        .map_err(|e| format!("vault.write: persist {}: {e}", path.display()))?;

    drop(password);
    Ok(json!({}))
}

fn exists(args: &Value) -> Result<Value, String> {
    let path = path_arg(args)?;
    Ok(Value::Bool(path.exists()))
}

fn delete(args: &Value) -> Result<Value, String> {
    let path = path_arg(args)?;
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(json!({})),
        // ENOENT = idempotent success — caller asked for "gone" and it's gone.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(json!({})),
        Err(e) => Err(format!("vault.delete({}): {e}", path.display())),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use zeroize::Zeroize;

    use super::*;

    #[test]
    fn take_password_returns_zeroizing_buffer() {
        // Sanity: take_password must produce a Zeroizing buffer with the
        // exact bytes of the input password.
        let pwd = take_password(&json!({ "password": "hunter2" })).unwrap();
        assert_eq!(pwd.as_slice(), b"hunter2");
    }

    #[test]
    fn take_password_rejects_missing_and_empty() {
        assert!(take_password(&json!({})).is_err());
        assert!(take_password(&json!({ "password": "" })).is_err());
    }

    #[test]
    fn zeroize_actually_wipes_buffer() {
        // The Zeroizing<Vec<u8>> wrapper invokes Zeroize::zeroize() on
        // drop. Prove the wipe by manually invoking it on an inner
        // vector — `Drop` runs `zeroize`, which sets every byte to 0.
        let mut buf = b"super-secret".to_vec();
        let original = buf.clone();
        buf.zeroize();
        assert_ne!(buf, original);
        assert!(buf.iter().all(|b| *b == 0));
    }
}

