//! Native plugin resolution: the configured plugins' `package.json`
//! declarations, validated into the crates a host build links.

use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    fs,
    path::{Path, PathBuf},
};

use anyhow::{bail, Context, Result};
use serde_json::Value;

use crate::{
    config::{permission_api, PluginEntry, KNOWN_APIS},
    project::discovery::resolve_package_dir,
};

#[derive(Clone, Debug)]
pub(super) struct NativeBuildPlugin {
    pub(super) package: String,
    pub(super) crate_name: String,
    /// The crate in the installed plugin package.
    pub(super) native_dir: PathBuf,
    /// The crate cargo compiles: `native_dir` until
    /// [`cache_native_plugins`](super::plugin_copy::cache_native_plugins)
    /// replaces it with its copy (in the host cache, or in `.tynd/build/plugins`
    /// for an editable plugin).
    pub(super) crate_dir: PathBuf,
    pub(super) features: Vec<String>,
    pub(super) permissions: Vec<String>,
    pub(super) required_apis: Vec<String>,
}

pub(super) fn resolve_native_build_plugins(
    cwd: &Path,
    entries: &[PluginEntry],
) -> Result<Vec<NativeBuildPlugin>> {
    let mut plugins = Vec::new();
    let mut seen_packages = HashSet::new();
    let mut crate_owners = BTreeMap::<String, String>::new();

    for entry in entries {
        let package = entry.package();
        if !seen_packages.insert(package.to_owned()) {
            bail!("plugin \"{package}\" is configured more than once");
        }
        let package_dir = resolve_package_dir(package, cwd)?.with_context(|| {
            format!("plugin \"{package}\" is not installed; run `bun add {package}`")
        })?;
        let manifest = read_package_manifest(&package_dir)?;
        let Some(native) = native_declaration(package, &manifest)? else {
            continue;
        };
        let native_dir = resolve_native_dir(package, &package_dir, native)?;
        let crate_name = resolve_plugin_crate_name(package, native)?;
        // Keyed by the Rust identifier: `a-b` and `a_b` are the same crate to rustc.
        if let Some(owner) = crate_owners.insert(crate_ident(&crate_name), package.to_owned()) {
            bail!("plugins \"{owner}\" and \"{package}\" both declare crateName \"{crate_name}\"");
        }

        let features = resolve_plugin_features(package, entry, native)?;
        let (permissions, required_apis) = resolve_plugin_permissions(package, entry, &manifest)?;
        plugins.push(NativeBuildPlugin {
            package: package.to_owned(),
            crate_name,
            crate_dir: native_dir.clone(),
            native_dir,
            features,
            permissions,
            required_apis,
        });
    }
    Ok(plugins)
}

pub(super) fn read_package_manifest(package_dir: &Path) -> Result<Value> {
    let manifest_path = package_dir.join("package.json");
    let manifest_source = fs::read_to_string(&manifest_path)
        .with_context(|| format!("read {}", manifest_path.display()))?;
    serde_json::from_str(&manifest_source)
        .with_context(|| format!("parse {}", manifest_path.display()))
}

/// The `package.json::tynd.native` object, or `None` when the plugin has no
/// native crate (key absent, `null` or `false`).
fn native_declaration<'a>(
    package: &str,
    manifest: &'a Value,
) -> Result<Option<&'a serde_json::Map<String, Value>>> {
    let Some(native_value) = manifest.pointer("/tynd/native") else {
        return Ok(None);
    };
    if native_value.is_null() || native_value == &Value::Bool(false) {
        return Ok(None);
    }
    native_value
        .as_object()
        .map(Some)
        .with_context(|| format!("plugin \"{package}\" declares invalid package.json::tynd.native"))
}

/// Canonical native crate directory (`native.path`, default `native`), which
/// must stay inside the package directory and hold a `Cargo.toml`.
fn resolve_native_dir(
    package: &str,
    package_dir: &Path,
    native: &serde_json::Map<String, Value>,
) -> Result<PathBuf> {
    let raw_native_path = native
        .get("path")
        .and_then(Value::as_str)
        .unwrap_or("native");
    if !valid_native_relative_path(raw_native_path) {
        bail!("plugin \"{package}\" declares invalid native.path \"{raw_native_path}\"");
    }
    let package_real = fs::canonicalize(package_dir)
        .with_context(|| format!("resolve plugin directory {}", package_dir.display()))?;
    let native_dir = package_dir.join(raw_native_path);
    let native_real = fs::canonicalize(&native_dir).with_context(|| {
        format!(
            "plugin \"{package}\" native.path cannot be resolved: {}",
            native_dir.display()
        )
    })?;
    if !native_real.starts_with(&package_real) {
        bail!("plugin \"{package}\" native.path escapes its package directory");
    }
    if !native_real.join("Cargo.toml").is_file() {
        bail!(
            "plugin \"{package}\" declares a native crate but Cargo.toml is missing at {}",
            native_real.display()
        );
    }
    Ok(native_real)
}

fn resolve_plugin_crate_name(
    package: &str,
    native: &serde_json::Map<String, Value>,
) -> Result<String> {
    let crate_name = native
        .get("crateName")
        .and_then(Value::as_str)
        .map_or_else(|| default_plugin_crate_name(package), str::to_owned);
    if !is_cargo_package_name(&crate_name) {
        bail!("plugin \"{package}\" declares invalid crateName \"{crate_name}\"");
    }
    Ok(crate_name)
}

fn resolve_plugin_features(
    package: &str,
    entry: &PluginEntry,
    native: &serde_json::Map<String, Value>,
) -> Result<Vec<String>> {
    let mut features = BTreeSet::new();
    if let Some(raw_features) = native.get("features") {
        let values = raw_features.as_array().with_context(|| {
            format!("plugin \"{package}\" tynd.native.features must be an array")
        })?;
        for feature in values {
            let feature = feature
                .as_str()
                .with_context(|| format!("plugin \"{package}\" has a non-string native feature"))?;
            validate_cargo_feature(package, feature)?;
            features.insert(feature.to_owned());
        }
    }
    for feature in entry.features() {
        validate_cargo_feature(package, feature)?;
        features.insert(feature.clone());
    }
    Ok(features.into_iter().collect())
}

fn resolve_plugin_permissions(
    package: &str,
    entry: &PluginEntry,
    manifest: &Value,
) -> Result<(Vec<String>, Vec<String>)> {
    let manifest_apis = manifest
        .pointer("/tynd/apis")
        .map(|value| parse_manifest_apis(package, value))
        .transpose()?
        .unwrap_or_default();
    let (configured_apis, configured_permissions) = match entry {
        PluginEntry::Package(_) => (&[][..], &[][..]),
        PluginEntry::Detailed(plugin) => (plugin.apis.as_slice(), plugin.permissions.as_slice()),
    };
    // Explicit permissions win; otherwise every configured (or, failing that,
    // manifest-declared) API is granted whole as `<api>:*`.
    let mut permissions = BTreeSet::new();
    let mut required_apis = BTreeSet::new();
    if configured_permissions.is_empty() {
        let fallback = if configured_apis.is_empty() {
            manifest_apis.as_slice()
        } else {
            configured_apis
        };
        for api in fallback {
            validate_host_api(package, api)?;
            permissions.insert(format!("{api}:*"));
            required_apis.insert(api.clone());
        }
    } else {
        for permission in configured_permissions {
            let api = permission_api(permission).with_context(|| {
                format!("plugin \"{package}\" declares invalid permission \"{permission}\"")
            })?;
            permissions.insert(permission.clone());
            required_apis.insert(api.to_owned());
        }
    }
    Ok((
        permissions.into_iter().collect(),
        required_apis.into_iter().collect(),
    ))
}

fn parse_manifest_apis(package: &str, value: &Value) -> Result<Vec<String>> {
    let values = value
        .as_array()
        .with_context(|| format!("plugin \"{package}\" tynd.apis must be an array"))?;
    let mut apis = BTreeSet::new();
    for value in values {
        let api = value
            .as_str()
            .with_context(|| format!("plugin \"{package}\" has a non-string tynd.apis entry"))?;
        validate_host_api(package, api)?;
        apis.insert(api.to_owned());
    }
    Ok(apis.into_iter().collect())
}

fn validate_host_api(package: &str, api: &str) -> Result<()> {
    if KNOWN_APIS.contains(&api) {
        Ok(())
    } else {
        bail!("plugin \"{package}\" declares unknown host API \"{api}\"")
    }
}

/// A `native.path` naming a directory inside the package: relative, made of
/// plain names and `.` (`./native` is common), with at least one name.
fn valid_native_relative_path(path: &str) -> bool {
    use std::path::Component;
    let components = || Path::new(path).components();
    !path.is_empty()
        && !Path::new(path).is_absolute()
        && path
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "_./-".contains(character))
        && components()
            .all(|component| matches!(component, Component::Normal(_) | Component::CurDir))
        && components().any(|component| matches!(component, Component::Normal(_)))
}

fn validate_cargo_feature(package: &str, feature: &str) -> Result<()> {
    if !feature.is_empty()
        && feature
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "_-".contains(character))
    {
        Ok(())
    } else {
        bail!("plugin \"{package}\" declares invalid Cargo feature \"{feature}\"")
    }
}

fn default_plugin_crate_name(package: &str) -> String {
    package
        .trim_start_matches('@')
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect()
}

/// Rust identifier of the Cargo package `crate_name`: Cargo maps `-` to `_`.
pub(super) fn crate_ident(crate_name: &str) -> String {
    crate_name.replace('-', "_")
}

/// A `crateName` is a Cargo package name (`tynd-plugin-audio`) whose Rust
/// identifier (`tynd_plugin_audio`) is not a keyword.
pub(super) fn is_cargo_package_name(value: &str) -> bool {
    const KEYWORDS: &[&str] = &[
        "as", "async", "await", "become", "box", "break", "const", "continue", "crate", "do",
        "dyn", "else", "enum", "extern", "false", "final", "fn", "for", "gen", "if", "impl", "in",
        "let", "loop", "macro", "match", "mod", "move", "mut", "override", "priv", "pub", "ref",
        "return", "self", "Self", "static", "struct", "super", "trait", "true", "try", "type",
        "typeof", "union", "unsafe", "unsized", "use", "virtual", "where", "while", "yield",
    ];
    !KEYWORDS.contains(&value)
        && value
            .chars()
            .next()
            .is_some_and(|character| character == '_' || character.is_ascii_lowercase())
        && value.chars().all(|character| {
            matches!(character, '_' | '-')
                || character.is_ascii_lowercase()
                || character.is_ascii_digit()
        })
}

#[cfg(test)]
mod tests;
