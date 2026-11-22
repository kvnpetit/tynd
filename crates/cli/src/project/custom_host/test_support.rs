//! Fixtures shared by the `custom_host` unit tests.

use std::{
    fs,
    path::{Path, PathBuf},
};

use serde_json::{json, Value};

use crate::config::TyndConfig;

use super::{
    plugins::NativeBuildPlugin,
    sources::{RustSourcePaths, HOST_SOURCES_PACKAGE},
};

pub(super) fn parse(value: Value) -> TyndConfig {
    serde_json::from_value(value).expect("config shape should deserialize")
}

pub(super) fn sources() -> RustSourcePaths {
    RustSourcePaths {
        runtime: PathBuf::from("/src/crates/runtime"),
        host: PathBuf::from("/src/crates/host"),
        plugin_sdk: PathBuf::from("/src/crates/plugin-sdk"),
    }
}

/// An installed `@tynd/host-src` at `version`; returns its `native` source root.
pub(super) fn installed_host_sources(root: &Path, version: &str) -> PathBuf {
    let package = root.join("node_modules/@tynd/host-src");
    let native = package.join("native");
    fs::create_dir_all(&native).expect("create host-src");
    fs::write(
        package.join("package.json"),
        json!({ "name": HOST_SOURCES_PACKAGE, "version": version }).to_string(),
    )
    .expect("write host-src manifest");
    native
}

/// An installed plugin (in `node_modules`) under `root`.
pub(super) fn native_plugin(root: &Path, source: &str) -> NativeBuildPlugin {
    plugin_crate_at(&root.join("node_modules/acme-plugin/native"), source)
}

pub(super) fn plugin_crate_at(native_dir: &Path, source: &str) -> NativeBuildPlugin {
    let native_dir = native_dir.to_path_buf();
    fs::create_dir_all(native_dir.join("src")).expect("create plugin crate");
    fs::create_dir_all(native_dir.join("target")).expect("create plugin target");
    fs::write(native_dir.join("src/lib.rs"), source).expect("write plugin source");
    NativeBuildPlugin {
        package: "acme-plugin".to_owned(),
        crate_name: "acme-plugin".to_owned(),
        crate_dir: native_dir.clone(),
        native_dir: fs::canonicalize(&native_dir).expect("canonical plugin crate"),
        features: Vec::new(),
        permissions: Vec::new(),
        required_apis: Vec::new(),
    }
}

pub(super) fn is_cache_key(key: &str) -> bool {
    key.len() == 12 && key.chars().all(|character| character.is_ascii_hexdigit())
}

/// `path` as the generated manifests write it.
pub(super) fn toml_path_value(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}
