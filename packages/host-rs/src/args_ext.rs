//! Typed argument extraction for OS-API dispatch.
//!
//! Every dispatch arm in `os/*` pulls fields out of a `serde_json::Value`
//! payload — typically `args.get("path").and_then(Value::as_str)
//! .ok_or_else(|| "fs.readText: missing 'path'".to_string())?`. The
//! pattern repeats ~80 times across the host. `ArgsExt` collapses it to
//! a one-liner that returns the same `Result<_, String>` the dispatcher
//! expects.
//!
//! Mirrors the trait shipped by `plugin-sdk` (which uses its own
//! `PluginError` type) — we keep them as twin traits rather than one
//! cross-crate trait so the host doesn't take a `PluginResult` runtime
//! dependency on a still-evolving SDK error enum.
//!
//! # Example
//!
//! ```ignore
//! use tynd_host::args_ext::ArgsExt;
//!
//! fn read_text(args: &serde_json::Value) -> Result<serde_json::Value, String> {
//!     let path = args.require::<String>("path", "fs", "readText")?;
//!     // ...
//! }
//! ```

use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::ipc_error::IpcError;

/// Extension trait that pulls typed fields out of a JSON argument bag.
/// Every method takes the API + method names so the resulting error
/// strings keep the `<api>.<method>: ...` shape the dispatcher
/// produces today.
pub trait ArgsExt {
    /// Pull a required field, deserialized into `T`. Returns
    /// `"<api>.<method>: missing '<key>' argument"` when absent and
    /// `"<api>.<method>: invalid '<key>' argument: <serde error>"` when
    /// the shape is wrong. The error is plain `String` so it drops
    /// straight into the dispatcher's `Result<Value, String>`.
    fn require<T: DeserializeOwned>(&self, key: &str, api: &str, method: &str)
        -> Result<T, String>;

    /// Pull an optional field. Returns `None` when the field is absent
    /// *or* fails to deserialize — callers that want to surface bad
    /// shapes should use [`require`](ArgsExt::require) instead.
    fn opt<T: DeserializeOwned>(&self, key: &str) -> Option<T>;

    /// Convenience for the very common `args.get("foo").and_then(Value::as_str)`
    /// shape — returns `&str` directly so the caller doesn't pay a
    /// `String` allocation when the value is only borrowed.
    fn require_str<'a>(&'a self, key: &str, api: &str, method: &str) -> Result<&'a str, String>;
}

impl ArgsExt for Value {
    fn require<T: DeserializeOwned>(
        &self,
        key: &str,
        api: &str,
        method: &str,
    ) -> Result<T, String> {
        let v = self
            .get(key)
            .ok_or_else(|| String::from(IpcError::missing_arg(api, method, key)))?;
        T::deserialize(v.clone())
            .map_err(|e| String::from(IpcError::new(api, method, format!("invalid '{key}': {e}"))))
    }

    fn opt<T: DeserializeOwned>(&self, key: &str) -> Option<T> {
        self.get(key).and_then(|v| T::deserialize(v.clone()).ok())
    }

    fn require_str<'a>(&'a self, key: &str, api: &str, method: &str) -> Result<&'a str, String> {
        self.get(key)
            .and_then(Value::as_str)
            .ok_or_else(|| String::from(IpcError::missing_arg(api, method, key)))
    }
}
