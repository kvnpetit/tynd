//! The app's native host, compiled from the Tynd Rust sources with the app's
//! host APIs and native plugins. [`build_custom_host`] is the entry point.

mod build;
mod fingerprint;
mod manifest;
mod plugin_copy;
mod plugins;
mod sources;
#[cfg(test)]
mod test_support;

pub(crate) use build::{build_custom_host, host_flamegraph_command};
pub(crate) use sources::{
    host_sources_available, host_sources_install_command, host_sources_spec, HOST_SOURCES_PACKAGE,
};
