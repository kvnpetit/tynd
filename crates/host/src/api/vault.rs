//! Encrypted file vault — AES-256-GCM via `cocoon` (default cipher,
//! never overridden in this codebase). Sized for secrets larger than
//! the OS keychain happily
//! holds: bundled API keys, plugin state, long refresh tokens, etc.
//!
//! Path handling delegates to the host's always-on path scope before
//! touching the disk; combined with cap-std roots, an unconfigured policy
//! still confines reads/writes to the
//! same scopes every other FS API uses. Passwords are accepted as
//! borrowed strings, never logged, and zeroized after use.

use std::{
    io::{Read as _, Write as _},
    path::{Path, PathBuf},
};

use base64::{engine::general_purpose::STANDARD, Engine as _};
use cocoon::Cocoon;
use serde_json::{json, Value};
use zeroize::Zeroizing;

use crate::api::HostApi;
use crate::ipc::error::IpcError;

// Crate-visible because handler macro emits public dispatch functions.
// Public methods are macro inputs; keeping the module crate-private prevents ACL bypass.
#[allow(unreachable_pub)]
pub(crate) mod inner {
    use serde_json::Value;

    use super::{delete_impl, exists_impl, read_impl, write_impl};
    use crate::tynd;

    /// Stateless dispatch marker.
    #[derive(Debug, Default)]
    pub(crate) struct VaultApi;

    #[tynd::handler(api = "vault")]
    impl VaultApi {
        /// Decrypt and read the vault file at `path` using `password`.
        /// Returns `{ data: <base64> }`.
        pub fn read(path: String, password: String) -> Result<Value, String> {
            read_impl(&path, &password)
        }

        /// Encrypt `data` (base64) with `password` and atomically
        /// write it to `path`. Creates parent directories as needed.
        pub fn write(path: String, password: String, data: Value) -> Result<Value, String> {
            write_impl(&path, &password, &data)
        }

        /// Check whether the vault file at `path` exists on disk.
        pub fn exists(path: String) -> Result<bool, String> {
            exists_impl(&path)
        }

        /// Delete the vault file at `path`. Idempotent — succeeds when
        /// the file is already absent.
        pub fn delete(path: String) -> Result<Value, String> {
            delete_impl(&path)
        }
    }
}

/// Row of the host API table (`api::HOST_APIS`).
pub(crate) const API: HostApi = HostApi::new("vault", |method, args| dispatch(method, &args));

/// Public dispatch wrapper — interposes the ACL gate before delegating
/// to the macro-generated table inside [`inner`].
pub(crate) fn dispatch(method: &str, args: &Value) -> Result<Value, String> {
    // ACL gate (additive on `security::check_fs`). Fail-open until a
    // capability manifest is loaded; then every `vault.*` is gated by
    // `vault:<method>`. See `crate::security::acl` for the capability shape.
    let cmd = format!("vault:{method}");
    if !crate::security::acl::authority().is_allowed(&cmd, None, None, None) {
        return Err(vault_err(method, "permission denied by ACL"));
    }
    inner::dispatch(method, args)
}

/// Build the `vault.<method>: <msg>` IPC error string.
#[cold]
fn vault_err(method: &str, msg: impl std::fmt::Display) -> String {
    IpcError::new("vault", method, msg).into_string()
}

/// Resolve a vault path through the cap-std confinement layer used by
/// the rest of `api::fs`. Mirrors `scheme_bin::route_write_binary` —
/// `path_scope::resolve` clamps the operation to
/// a pre-opened root `Dir`. The returned `(dir, rel)` is the *only*
/// surface vault uses for I/O so a path that lex-cleans outside every
/// allowed root can't slip past via raw ambient filesystem calls.
fn resolved_path(
    api_method: &str,
    path_str: &str,
) -> Result<(&'static cap_std::fs::Dir, PathBuf), String> {
    crate::security::path_scope::resolve(path_str).map_err(|e| vault_err(api_method, e))
}

/// Wrap a password string into a `Zeroizing` buffer so the bytes are
/// wiped from memory the moment the buffer goes out of scope.
/// `Vec::drop` only releases the heap allocation — it doesn't scrub
/// the page, so a future allocator handing the same memory back can
/// leak the cleartext. The original `serde_json::Value` still owns a
/// copy until it's dropped at the dispatch layer (outside our
/// control), but that bounds the residency window for our copy.
fn take_password(p: &str) -> Result<Zeroizing<Vec<u8>>, String> {
    if p.is_empty() {
        return Err(vault_err("*", "'password' must not be empty"));
    }
    Ok(Zeroizing::new(p.as_bytes().to_vec()))
}

/// Largest vault file `read` accepts (1 GiB).
const MAX_VAULT_BYTES: u64 = 1024 * 1024 * 1024;

fn read_impl(path_str: &str, password_str: &str) -> Result<Value, String> {
    let (dir, rel) = resolved_path("read", path_str)?;
    let password = take_password(password_str)?;

    // Pull the ciphertext through the capability-confined `Dir` so the
    // open syscall is bounded by the same cap-std root every other FS
    // API uses. The read is capped so an oversized file can't exhaust
    // memory, and `Cocoon::unwrap` checks the header's claimed length
    // against the bytes actually present (`Cocoon::parse` would allocate
    // whatever a forged header claims and abort the process).
    let io_err = |e: std::io::Error| vault_err("read", format!("({path_str}): {e}"));
    let file = dir.open(&rel).map_err(io_err)?;
    let mut bytes = Vec::new();
    file.take(MAX_VAULT_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(io_err)?;
    if bytes.len() as u64 > MAX_VAULT_BYTES {
        return Err(vault_err(
            "read",
            format!("({path_str}): file exceeds the {MAX_VAULT_BYTES}-byte cap"),
        ));
    }
    let cocoon = Cocoon::new(&password);
    let plaintext = cocoon
        .unwrap(&bytes)
        .map_err(|e| vault_err("read", format!("({path_str}): parse: {e:?}")))?;
    // Wrap the decrypted bytes in `Zeroizing` so the heap allocation is
    // scrubbed before release. Without this the freed slab still holds
    // plaintext until reused — the whole point of vault is to shorten
    // the residency window for secrets.
    let plaintext = Zeroizing::new(plaintext);

    // Drop the key material as soon as the parse returns. The `Zeroizing`
    // wrapper scrubs the buffer before releasing the allocation; `cocoon`
    // itself pulls in `zeroize` for its internal scratch (verified via
    // lockfile). This just shortens our copy's lifetime.
    drop(password);

    Ok(json!({
        "data": STANDARD.encode(&*plaintext),
    }))
}

/// Decode the `write` payload: a base64 string (canonical form, matching
/// `read()`'s return shape) or a JSON array of byte values.
fn decode_write_data(data: &Value) -> Result<Vec<u8>, String> {
    match data {
        Value::String(b64) => STANDARD
            .decode(b64.as_bytes())
            .map_err(|e| vault_err("write", format!("base64 decode: {e}"))),
        Value::Array(arr) => arr
            .iter()
            .map(|v| {
                v.as_u64()
                    .and_then(|n| u8::try_from(n).ok())
                    .ok_or_else(|| vault_err("write", "non-byte element in 'data'"))
            })
            .collect(),
        _ => Err(vault_err(
            "write",
            "missing 'data' (base64 string or byte array)",
        )),
    }
}

/// Sibling temp path `<basename>.tmp.<pid>.<nanos>` — deterministic but
/// unique, so no ambient FS access is needed just to mkstemp.
fn temp_sibling(rel: &Path) -> PathBuf {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let mut name = rel
        .file_name()
        .map(std::ffi::OsStr::to_os_string)
        .unwrap_or_default();
    name.push(format!(".tmp.{}.{nonce}", std::process::id()));
    rel.with_file_name(name)
}

/// Create `tmp_rel` inside `dir`, write `cipher`, restrict permissions
/// (Unix) and fsync.
fn write_temp_file(dir: &cap_std::fs::Dir, tmp_rel: &Path, cipher: &[u8]) -> Result<(), String> {
    let mut f = dir
        .create(tmp_rel)
        .map_err(|e| vault_err("write", format!("tempfile {}: {e}", tmp_rel.display())))?;
    f.write_all(cipher)
        .map_err(|e| vault_err("write", format!("write {}: {e}", tmp_rel.display())))?;
    // Tighten Unix perms to user-only — the default umask on most
    // distros is `0644`, which lets every other local user read the
    // ciphertext and try offline password attacks. Windows skips:
    // files under `%APPDATA%` inherit the user-only parent ACL
    // already, mirroring `api::updater::write_staged_blob`.
    #[cfg(unix)]
    {
        use cap_std::fs::PermissionsExt as _;
        let perms = cap_std::fs::Permissions::from_mode(0o600);
        f.set_permissions(perms)
            .map_err(|e| vault_err("write", format!("chmod 0600: {e}")))?;
    }
    f.sync_all()
        .map_err(|e| vault_err("write", format!("fsync: {e}")))
}

fn write_impl(path_str: &str, password_str: &str, data: &Value) -> Result<Value, String> {
    let (dir, rel) = resolved_path("write", path_str)?;
    let password = take_password(password_str)?;
    let data_bytes = decode_write_data(data)?;

    // Materialise parent dirs through the confined `Dir` so a missing
    // chain like `~/data/foo/bar.vault` still gets created without
    // ever escaping the capability root.
    if let Some(parent) = rel.parent() {
        if !parent.as_os_str().is_empty() {
            dir.create_dir_all(parent)
                .map_err(|e| vault_err("write", format!("mkdir {}: {e}", parent.display())))?;
        }
    }

    // Encrypt into a heap buffer first, then atomically rename a
    // sibling tempfile into place. cap-std's `Dir::rename` is atomic on
    // POSIX and on Windows >= 10 (MoveFileEx), so a crash never leaves
    // a half-written vault behind.
    let mut cipher = Vec::new();
    let mut cocoon = Cocoon::new(&password);
    cocoon
        .dump(data_bytes, &mut cipher)
        .map_err(|e| vault_err("write", format!("({path_str}): dump: {e:?}")))?;
    // Same cap as `read_impl`, so every vault we write can be read back.
    if cipher.len() as u64 > MAX_VAULT_BYTES {
        return Err(vault_err(
            "write",
            format!("({path_str}): data exceeds the {MAX_VAULT_BYTES}-byte cap"),
        ));
    }

    let tmp_rel = temp_sibling(&rel);
    write_temp_file(dir, &tmp_rel, &cipher)?;
    if let Err(e) = dir.rename(&tmp_rel, dir, &rel) {
        // Best-effort cleanup so a failed rename doesn't leave the
        // tempfile lingering inside the user's data dir.
        let _ = dir.remove_file(&tmp_rel);
        return Err(vault_err(
            "write",
            format!("rename {} -> {}: {e}", tmp_rel.display(), rel.display()),
        ));
    }

    drop(password);
    Ok(json!({}))
}

fn exists_impl(path_str: &str) -> Result<bool, String> {
    let (dir, rel) = resolved_path("exists", path_str)?;
    Ok(dir.try_exists(&rel).unwrap_or(false))
}

fn delete_impl(path_str: &str) -> Result<Value, String> {
    let (dir, rel) = resolved_path("delete", path_str)?;
    match dir.remove_file(&rel) {
        Ok(()) => Ok(json!({})),
        // ENOENT = idempotent success — caller asked for "gone" and it's gone.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(json!({})),
        Err(e) => Err(vault_err("delete", e)),
    }
}

#[cfg(test)]
mod tests;
