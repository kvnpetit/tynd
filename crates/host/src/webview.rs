//! Everything a webview is built from: the builder steps, the wry handlers,
//! navigation rules, the `tynd://` and `tynd-bin://` schemes, the plugin URI
//! schemes and the JS shim injected into every page.

pub(crate) mod builder;
pub(crate) mod handlers;
pub(crate) mod navigation;
#[cfg(feature = "plugins")]
pub(crate) mod plugin_handlers;
pub(crate) mod scheme;
pub(crate) mod scheme_bin;
pub(crate) mod shim;
