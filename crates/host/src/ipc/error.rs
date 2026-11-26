//! Builder for the `Result<_, String>` error strings every OS API dispatch
//! arm produces (`"<api>.<method>: <msg>"`).
//!
//! ```ignore
//! let path = args.get("path")
//!     .and_then(Value::as_str)
//!     .ok_or_else(|| IpcError::missing_arg("fs", "readText", "path").into())?;
//! ```

use std::fmt::Display;

/// `<api>.<method>: <msg>` error newtype. Converts into `String` at the IPC
/// boundary (every dispatch arm returns `Result<Value, String>`).
#[derive(Debug, Clone)]
pub(crate) struct IpcError(String);

impl IpcError {
    /// Build `"<api>.<method>: <msg>"`. `#[cold]` keeps the format
    /// machinery out of the hot icache.
    #[cold]
    pub(crate) fn new(api: &str, method: &str, msg: impl Display) -> Self {
        Self(format!("{api}.{method}: {msg}"))
    }

    /// "missing '<arg>' argument" shape.
    #[cold]
    pub(crate) fn missing_arg(api: &str, method: &str, arg: &str) -> Self {
        Self::new(api, method, format!("missing '{arg}' argument"))
    }

    /// Trailing `_ => Err(...)` arm shape.
    #[cold]
    pub(crate) fn unknown_method(api: &str, method: &str) -> Self {
        Self::new(api, method, "unknown method")
    }

    /// Consume into the inner `String`. Reads better than `Into::into`
    /// inside `map_err(|e| ...)` closures where inference is ambiguous.
    pub(crate) fn into_string(self) -> String {
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
