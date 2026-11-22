//! Secure credential storage backed by the OS keychain.
//!
//! Backends: macOS Keychain, Windows Credential Manager + DPAPI, Linux
//! Secret Service / GNOME Keyring / KWallet.
//!
//! Entries are scoped by `(service, account)`.

use keyring::Entry;
use zeroize::Zeroizing;

use crate::api::HostApi;
use crate::ipc::error::IpcError;
use crate::tynd;

const MAX_SERVICE_CHARS: usize = 256;
const MAX_ACCOUNT_CHARS: usize = 256;
const MAX_PASSWORD_BYTES: usize = 8 * 1024;

/// Marker type for the `#[tynd::handler]` dispatch table.
#[derive(Debug, Default)]
pub(crate) struct KeyringApi;

/// Row of the host API table (`api::HOST_APIS`).
pub(crate) const API: HostApi = HostApi::new("keyring", |method, args| dispatch(method, &args));

#[tynd::handler(api = "keyring")]
impl KeyringApi {
    /// Store `password` under `(service, account)`. Returns `null` on success.
    pub fn set(service: String, account: String, password: String) -> Result<(), String> {
        // SECURITY: parity with `vault` — wrap the inbound password so its
        // bytes are zeroed on drop. Without this, the heap allocation is
        // returned to the allocator with the cleartext intact and a later
        // allocation could snapshot it. `Zeroizing<String>` runs
        // `String::zeroize()` (= zero every byte then set len = 0) on drop.
        let password = Zeroizing::new(password);
        validate_password(password.as_str())?;
        new_entry("set", &service, &account)?
            .set_password(password.as_str())
            .map_err(|e| IpcError::new("keyring", "set", e).into_string())
    }

    /// Read the password under `(service, account)`. `null` when absent
    /// (normal first-run case, not an error).
    ///
    /// SECURITY — residual cleartext window: parity with `set` zeroes
    /// the `keyring` crate's `String` on this fn's exit paths, but the
    /// `as_str().to_owned()` clone produces a plain `String` the IPC
    /// serializer moves into the JSON response. That clone is *not*
    /// zeroed — the JSON encoder hands the bytes off to the wire layer,
    /// which copies them again into a frame buffer, and at no point in
    /// that chain does anything implement `ZeroizeOnDrop`. So a fresh
    /// allocator snapshot taken between the IPC return and the wire
    /// flush could still observe the password in memory. Eliminating
    /// that window would require teaching the IPC encoder to zero
    /// `String` values flagged as secret, which is a larger refactor;
    /// for now we accept the residual exposure and document it here so
    /// callers know `keyring.get()` is *not* a hardened secret-reading
    /// primitive — for high-stakes secrets prefer `vault` (which keeps
    /// material inside `Zeroizing` end-to-end on the host side).
    pub fn get(service: String, account: String) -> Result<Option<String>, String> {
        match new_entry("get", &service, &account)?.get_password() {
            Ok(s) => {
                let z = Zeroizing::new(s);
                Ok(Some(z.as_str().to_owned()))
            },
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(IpcError::new("keyring", "get", e).into_string()),
        }
    }

    /// Delete the entry under `(service, account)`. `true` when removed,
    /// `false` when absent.
    pub fn delete(service: String, account: String) -> Result<bool, String> {
        match new_entry("delete", &service, &account)?.delete_credential() {
            Ok(()) => Ok(true),
            Err(keyring::Error::NoEntry) => Ok(false),
            Err(e) => Err(IpcError::new("keyring", "delete", e).into_string()),
        }
    }
}

/// Build a `keyring::Entry` with shared validation. `method` is threaded
/// in so error paths produce the canonical `keyring.<method>: ...` shape.
fn new_entry(method: &str, service: &str, account: &str) -> Result<Entry, String> {
    validate_identifier(method, "service", service, MAX_SERVICE_CHARS)?;
    validate_identifier(method, "account", account, MAX_ACCOUNT_CHARS)?;
    Entry::new(service, account).map_err(|e| IpcError::new("keyring", method, e).into_string())
}

fn validate_identifier(
    method: &str,
    field: &str,
    value: &str,
    max_chars: usize,
) -> Result<(), String> {
    let invalid = if value.is_empty() {
        Some("must not be empty".to_string())
    } else if value.contains('\0') {
        Some("must not contain NUL bytes".to_string())
    } else {
        let chars = value.chars().count();
        (chars > max_chars).then(|| format!("is {chars} characters; maximum is {max_chars}"))
    };
    match invalid {
        Some(detail) => {
            Err(IpcError::new("keyring", method, format!("'{field}' {detail}")).into_string())
        },
        None => Ok(()),
    }
}

fn validate_password(password: &str) -> Result<(), String> {
    if password.len() > MAX_PASSWORD_BYTES {
        return Err(IpcError::new(
            "keyring",
            "set",
            format!(
                "'password' is {} bytes; maximum is {MAX_PASSWORD_BYTES}",
                password.len()
            ),
        )
        .into_string());
    }
    if password.contains('\0') {
        return Err(
            IpcError::new("keyring", "set", "'password' must not contain NUL bytes").into_string(),
        );
    }
    Ok(())
}
