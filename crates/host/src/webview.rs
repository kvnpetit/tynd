//! Everything a webview is built from: the builder steps, the wry handlers,
//! navigation rules, the `tynd://` and `tynd-bin://` schemes, the plugin URI
//! schemes and the JS shim injected into every page.

pub(crate) mod builder;
pub(crate) mod handlers;
pub(crate) mod navigation;
pub(crate) mod permissions;
#[cfg(feature = "plugins")]
pub(crate) mod plugin_handlers;
pub(crate) mod process_failed;
pub(crate) mod protocols;
pub(crate) mod scheme;
pub(crate) mod scheme_bin;
pub(crate) mod session;
pub(crate) mod shim;
