use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::{PluginError, PluginResult};

/// Extension trait on [`serde_json::Value`] for pulling typed fields out of plugin args.
pub trait ArgsExt {
    /// Pull a required field, deserialized to `T`.
    fn require<T: DeserializeOwned>(&self, key: &str) -> PluginResult<T>;

    /// Pull an optional field. Returns `None` when absent or on deserialize failure.
    fn opt<T: DeserializeOwned>(&self, key: &str) -> Option<T>;
}

impl ArgsExt for Value {
    fn require<T: DeserializeOwned>(&self, key: &str) -> PluginResult<T> {
        let v = self.get(key).ok_or_else(|| PluginError::MissingArg {
            key: key.to_string(),
        })?;
        T::deserialize(v).map_err(|e| PluginError::InvalidArg {
            key: key.to_string(),
            detail: e.to_string(),
        })
    }

    fn opt<T: DeserializeOwned>(&self, key: &str) -> Option<T> {
        self.get(key).and_then(|v| T::deserialize(v).ok())
    }
}
