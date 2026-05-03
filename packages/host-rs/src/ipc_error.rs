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
    ///
    /// `#[cold]` (B21): every constructor here lives on the IPC error
    /// path, which is by definition off the hot dispatch path. Marking
    /// these cold helps the compiler keep the format machinery out of
    /// the hot icache lines that the success branches occupy.
    #[cold]
    pub fn new(api: &str, method: &str, msg: impl Display) -> Self {
        Self(format!("{api}.{method}: {msg}"))
    }

    /// Common shape for "the JSON `args` payload is missing the named
    /// argument". Drops the trailing `'arg'` quoting that a hand-rolled
    /// `format!` would otherwise leave inconsistent across sites.
    #[cold]
    pub fn missing_arg(api: &str, method: &str, arg: &str) -> Self {
        Self::new(api, method, format!("missing '{arg}' argument"))
    }

    /// Catch-all for the trailing `_ => Err(...)` arm in every dispatch
    /// table. Keeps the unknown-method shape uniform across APIs.
    #[cold]
    pub fn unknown_method(api: &str, method: &str) -> Self {
        Self::new(api, method, "unknown method")
    }

    /// Borrow the inner message — handy for `assert!(err.contains(...))`
    /// in tests without paying a `to_string()` round trip.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Consume and return the inner `String`. Equivalent to
    /// `String::from(err)` but reads as `err.into_string()` at call
    /// sites that already have a typed `IpcError` in hand — avoids the
    /// `Into::into` inference dance when the surrounding context is a
    /// generic closure (`map_err(|e| ...)`) where multiple `From` impls
    /// would otherwise need a turbofish.
    pub fn into_string(self) -> String {
        self.0
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
