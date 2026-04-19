//! Secure credential storage backed by the OS keychain.
//!
//! Platform backends (selected via `keyring` feature flags at build time):
//! - macOS: Apple Keychain (`apple-native`)
//! - Windows: Credential Manager + DPAPI (`windows-native`)
//! - Linux: Secret Service / GNOME Keyring / KWallet (`sync-secret-service`)
//!
//! Entries are scoped by `(service, account)`. `service` is typically the
//! app's reverse-DNS identifier (same shape as `singleInstance.acquire`);
//! `account` distinguishes multiple credentials for the same service
//! (e.g. different users, API keys).

use keyring::Entry;
use serde_json::Value;

pub fn dispatch(method: &str, args: &Value) -> Result<Value, String> {
    match method {
        "set" => set_secret(args),
        "get" => get_secret(args),
        "delete" => delete_secret(args),
        _ => Err(format!("keyring.{method}: unknown method")),
    }
}

fn entry(args: &Value) -> Result<Entry, String> {
    let service = args
        .get("service")
        .and_then(Value::as_str)
        .ok_or_else(|| "keyring: missing 'service'".to_string())?;
    let account = args
        .get("account")
        .and_then(Value::as_str)
        .ok_or_else(|| "keyring: missing 'account'".to_string())?;
    Entry::new(service, account).map_err(|e| format!("keyring: {e}"))
}

fn set_secret(args: &Value) -> Result<Value, String> {
    let password = args
        .get("password")
        .and_then(Value::as_str)
        .ok_or_else(|| "keyring.set: missing 'password'".to_string())?;
    entry(args)?
        .set_password(password)
        .map_err(|e| format!("keyring.set: {e}"))?;
    Ok(Value::Null)
}

fn get_secret(args: &Value) -> Result<Value, String> {
    match entry(args)?.get_password() {
        Ok(s) => Ok(Value::String(s)),
        // "not found" is a normal case — return null instead of erroring.
        Err(keyring::Error::NoEntry) => Ok(Value::Null),
        Err(e) => Err(format!("keyring.get: {e}")),
    }
}

fn delete_secret(args: &Value) -> Result<Value, String> {
    match entry(args)?.delete_credential() {
        Ok(()) => Ok(Value::Bool(true)),
        Err(keyring::Error::NoEntry) => Ok(Value::Bool(false)),
        Err(e) => Err(format!("keyring.delete: {e}")),
    }
}
