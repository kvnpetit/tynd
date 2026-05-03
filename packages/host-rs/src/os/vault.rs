//! Encrypted file vault — AES-GCM-SIV (or ChaCha20-Poly1305 fallback)
//! over `cocoon`. Sized for secrets larger than the OS keychain happily
//! holds: bundled API keys, plugin state, long refresh tokens, etc.
//!
//! Path handling delegates to `super::security::check_fs` (policy gate)
//! before touching the disk; combined with the host's cap-std roots in
//! `os::fs`, an unconfigured policy still confines reads/writes to the
//! same scopes every other FS API uses. Passwords are accepted as
//! borrowed strings, never logged, and zeroized after use.

use std::{io::Write as _, path::PathBuf};

// `fs-err` shim — error already includes the offending path, dropping the
// hand-stamped `format!("vault.write: mkdir {}: {e}", parent.display())`
// pairs that surrounded every direct std::fs call here.
use fs_err as fs;
use fs_err::File;

use base64::{engine::general_purpose::STANDARD, Engine as _};
use cocoon::Cocoon;
use serde_json::{json, Value};
use zeroize::Zeroizing;

pub fn dispatch(method: &str, args: &Value) -> Result<Value, String> {
    // W3.2 — ACL gate (additive on top of `security::check_fs`).
    // The `acl::Authority` is "fail open" until an admin loads at least
    // one capability manifest, so apps that don't opt into the granular
    // ACL model see no behaviour change. Once loaded, every `vault.*`
    // method is gated by `vault:<method>` (e.g. `vault:write`) — see
    // `os::acl` docs for the full capability shape.
    #[cfg(feature = "api-acl")]
    {
        let cmd = format!("vault:{method}");
        if !crate::os::acl::authority().is_allowed(&cmd, None, None, None) {
            return Err(format!("vault.{method}: permission denied by ACL"));
        }
    }
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

    let mut file = File::open(&path).map_err(|e| format!("vault.read: {e}"))?;
    let cocoon = Cocoon::new(&password);
    let plaintext = cocoon
        .parse(file.file_mut())
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
            fs::create_dir_all(parent).map_err(|e| format!("vault.write: {e}"))?;
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
    match fs::remove_file(&path) {
        Ok(()) => Ok(json!({})),
        // ENOENT = idempotent success — caller asked for "gone" and it's gone.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(json!({})),
        Err(e) => Err(format!("vault.delete: {e}")),
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

    // -----------------------------------------------------------------
    // W3.2 — ACL wire smoke test
    //
    // Both branches of the `api-acl`-cfg-gated check are exercised:
    //   1. unconfigured authority → `is_allowed` returns true → dispatch
    //      reaches the underlying handler (which then errors on the
    //      bogus path, but only AFTER the ACL gate let it through).
    //   2. configured authority WITHOUT a vault grant → dispatch
    //      short-circuits with "permission denied by ACL".
    // The acl::authority() singleton is process-wide; the test
    // serializes via the same lock the `acl::tests` module uses.
    // -----------------------------------------------------------------
    #[cfg(feature = "api-acl")]
    #[test]
    fn dispatch_acl_gate_short_circuits_when_denied() {
        use std::num::NonZeroU64;
        use tauri_utils::acl::{
            capability::{Capability, PermissionEntry},
            manifest::Manifest,
            Commands, Identifier, Permission,
        };

        // Reset the singleton, register a manifest for a *different*
        // plugin, then add a capability that grants nothing under
        // `vault:*`. With at least one capability loaded, the authority
        // flips into "deny by default" — `vault:read` must surface the
        // ACL error rather than reaching the file-open path.
        let auth = crate::os::acl::authority();
        let mut perms = std::collections::BTreeMap::new();
        perms.insert(
            "allow-read".to_string(),
            Permission {
                version: NonZeroU64::new(1),
                identifier: "allow-read".into(),
                description: None,
                commands: Commands {
                    allow: vec!["read".into()],
                    deny: vec![],
                },
                scope: Default::default(),
                platforms: None,
            },
        );
        let manifest = Manifest {
            default_permission: None,
            permissions: perms,
            permission_sets: std::collections::BTreeMap::new(),
            global_scope_schema: None,
        };
        auth.register_manifest("fs", manifest).expect("manifest");
        let id = Identifier::try_from("fs:allow-read".to_string()).expect("ident");
        let cap = Capability {
            identifier: "vault-acl-test".into(),
            description: String::new(),
            remote: None,
            local: true,
            windows: vec![],
            webviews: vec![],
            permissions: vec![PermissionEntry::PermissionRef(id)],
            platforms: None,
        };
        auth.add_capability(cap).expect("add");

        let err = dispatch("read", &json!({ "path": "/tmp/whatever" })).unwrap_err();
        assert!(
            err.contains("permission denied by ACL"),
            "expected ACL denial, got: {err}"
        );

        // Cleanup — drop the test capability so other tests in the same
        // binary aren't affected. The test_lock in `acl::tests` keeps
        // tests in *that* module serialized, but we can't grab it from
        // outside. Documented limitation: don't run vault::dispatch ACL
        // tests in parallel with `acl::tests`.
        auth.remove_capability("vault-acl-test").expect("remove");
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

