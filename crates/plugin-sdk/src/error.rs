use thiserror::Error;

/// Typed error returned from plugin methods.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum PluginError {
    /// A required argument was absent on the incoming `args` JSON.
    #[error("missing required arg '{key}'")]
    MissingArg {
        /// JSON key that was expected but missing.
        key: String,
    },

    /// An argument was present but failed to deserialize into the requested type.
    #[error("invalid arg '{key}': {detail}")]
    InvalidArg {
        /// JSON key that failed to deserialize.
        key: String,
        /// Underlying serde / parse error formatted as a string.
        detail: String,
    },

    /// The whole `args` JSON blob failed to deserialize into a method's generated args struct.
    #[error("invalid args for method '{method}': {detail}")]
    InvalidArgsBlob {
        /// Wire method name whose args struct rejected the blob.
        method: String,
        /// Underlying serde / parse error formatted as a string.
        detail: String,
    },

    /// A plugin tried to reach an OS command outside its declared permission allow-list.
    #[error(
        "plugin capability: not permitted to call {api}.{method} (declared permissions: {allowed:?})"
    )]
    NotPermitted {
        /// API name the plugin attempted to reach.
        api: String,
        /// Method name on that API.
        method: String,
        /// Command selectors the plugin was instantiated with.
        allowed: Vec<String>,
    },

    /// `cx.os_call(...)` was invoked from a context that has no host dispatcher attached.
    #[error("plugin capability: host dispatch not wired")]
    DispatchNotWired,

    /// A plugin's `dispatch` fallthrough.
    #[error("{plugin}.{method}: unknown method")]
    UnknownMethod {
        /// Plugin name.
        plugin: String,
        /// Method that was requested but not handled.
        method: String,
    },

    /// Free-form plugin error.
    #[error("{0}")]
    Custom(String),

    /// I/O error bubbled up from `std::io`.
    #[error(transparent)]
    Io(#[from] std::io::Error),

    /// JSON serialization or deserialization error.
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

impl From<String> for PluginError {
    fn from(s: String) -> Self {
        Self::Custom(s)
    }
}

impl From<&str> for PluginError {
    fn from(s: &str) -> Self {
        Self::Custom(s.to_string())
    }
}

impl From<Box<dyn std::error::Error + Send + Sync>> for PluginError {
    fn from(e: Box<dyn std::error::Error + Send + Sync>) -> Self {
        Self::Custom(e.to_string())
    }
}

/// `Result` alias carrying [`PluginError`].
pub type PluginResult<T> = Result<T, PluginError>;

/// Canonical "unknown method" error for plugin `dispatch` fallthrough arms.
#[must_use]
pub fn unknown_method(plugin: &str, method: &str) -> PluginError {
    PluginError::UnknownMethod {
        plugin: plugin.to_string(),
        method: method.to_string(),
    }
}
