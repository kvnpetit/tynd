//! Compact builder for the `Result<_, String>` error strings every OS
//! API dispatch arm produces.
//!
//! The IPC wire format is plain `String`, so we don't pull `thiserror` or
//! a structured error enum — every error round-trips through one
//! `format!("{api}.{method}: {message}")` call. The hand-rolled
//! `format!` chain showed up in 250+ sites across `os/*`, each one a
//! near-identical four-token string. `IpcError` collapses the boilerplate
//! into a single typed builder while preserving the exact message shape
//! callers (and existing tests) already pin against.
//!
//! # Example
//!
//! Before:
//! ```ignore
//! let path = args.get("path")
//!     .and_then(Value::as_str)
//!     .ok_or_else(|| "fs.readText: missing 'path'".to_string())?;
//! ```
//!
//! After:
//! ```ignore
//! let path = args.get("path")
//!     .and_then(Value::as_str)
//!     .ok_or_else(|| IpcError::missing_arg("fs", "readText", "path").into())?;
//! ```
//!
//! The builder also surfaces `unknown_method(...)` for the trailing
//! `_ => Err(...)` arms in every dispatch table so the message shape
//! stays uniform.

use std::fmt::Display;

/// Newtype around the user-facing error string. Constructed via the
/// builder methods below; converts into `String` for free at the IPC
/// boundary (every dispatch arm returns `Result<Value, String>`).
///
/// The inner string layout is intentional: `<api>.<method>: <msg>`
/// matches what the previous hand-rolled `format!`s emitted, so error
/// assertion strings in callers + tests don't have to change.
#[derive(Debug, Clone)]
pub struct IpcError(String);

impl IpcError {
    /// Build a fully-qualified error: `"<api>.<method>: <msg>"`.
    pub fn new(api: &str, method: &str, msg: impl Display) -> Self {
        Self(format!("{api}.{method}: {msg}"))
    }

    /// Common shape for "the JSON `args` payload is missing the named
    /// argument". Drops the trailing `'arg'` quoting that a hand-rolled
    /// `format!` would otherwise leave inconsistent across sites.
    pub fn missing_arg(api: &str, method: &str, arg: &str) -> Self {
        Self::new(api, method, format!("missing '{arg}' argument"))
    }

    /// Catch-all for the trailing `_ => Err(...)` arm in every dispatch
    /// table. Keeps the unknown-method shape uniform across APIs.
    pub fn unknown_method(api: &str, method: &str) -> Self {
        Self::new(api, method, "unknown method")
    }

    /// Borrow the inner message — handy for `assert!(err.contains(...))`
    /// in tests without paying a `to_string()` round trip.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<IpcError> for String {
    fn from(e: IpcError) -> String {
        e.0
    }
}

impl Display for IpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
