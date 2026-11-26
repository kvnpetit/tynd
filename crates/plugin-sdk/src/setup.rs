use std::sync::Arc;

/// Request to a plugin-registered custom URI scheme.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UriRequest {
    /// Full request URL (`scheme://host/path?query`).
    pub url: String,
    /// HTTP method.
    pub method: String,
    /// Request headers in arrival order.
    pub headers: Vec<(String, String)>,
    /// Raw request body.
    pub body: Vec<u8>,
    /// Label of the webview that issued the request.
    pub webview: String,
    /// Origin of the requesting page: `None` for bundled app content,
    /// `Some("https://example.com")` for anything else. Always `None` or a
    /// loopback dev origin unless the scheme was registered with
    /// [`crate::PluginCx::register_uri_scheme_remote`].
    pub origin: Option<String>,
}

/// Response returned by a custom URI scheme handler.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UriResponse {
    /// HTTP status code.
    pub status: u16,
    /// Response headers (`Content-Type` included).
    pub headers: Vec<(String, String)>,
    /// Raw response body.
    pub body: Vec<u8>,
}

/// Handler invoked for every request on a plugin-registered scheme.
pub type UriHandler = Arc<dyn Fn(UriRequest) -> UriResponse + Send + Sync>;

/// Host-implemented sink behind [`crate::PluginCx::register_init_script`] and
/// [`crate::PluginCx::register_uri_scheme`].
pub trait PluginSetup: Send + Sync {
    /// Queue `script` to run in every webview created afterwards.
    fn init_script(&self, script: String) -> Result<(), String>;

    /// Register a custom URI scheme.
    fn uri_scheme(&self, name: String, handler: UriHandler) -> Result<(), String>;

    /// Like [`Self::init_script`]; `remote: true` also runs the script in
    /// webviews showing remote content. The default ignores `remote` and keeps
    /// the script local-only, the safe direction for older sinks.
    fn init_script_scoped(&self, script: String, remote: bool) -> Result<(), String> {
        let _ = remote;
        self.init_script(script)
    }

    /// Like [`Self::uri_scheme`]; `remote: true` also serves requests from
    /// remote pages. The default ignores `remote` (local-only).
    fn uri_scheme_scoped(
        &self,
        name: String,
        handler: UriHandler,
        remote: bool,
    ) -> Result<(), String> {
        let _ = remote;
        self.uri_scheme(name, handler)
    }
}
