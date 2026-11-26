//! Generated files of the host workspace: the app `Cargo.toml` and
//! `main.rs`, and the workspace manifest around each plugin copy.

use std::{
    collections::{BTreeSet, HashSet},
    fmt::Write as _,
    fs,
    io::Write as _,
    path::Path,
};

use anyhow::{bail, Context, Result};

use crate::config::{resolve_observability, TyndConfig, CORE_HOST_APIS};

use super::{
    plugin_copy::{LOCAL_PLUGINS_DIR, PLUGIN_COPY_CRATE_DIR},
    plugins::{crate_ident, NativeBuildPlugin},
    sources::{collect_native_files, RustSourcePaths},
};

/// Telemetry crates are compiled in only for the pipelines the config turns on,
/// so apps without `observability` do not pay for them in binary size.
pub(super) fn observability_host_features(config: &TyndConfig) -> Vec<String> {
    let Some(observability) = resolve_observability(config) else {
        return Vec::new();
    };
    let mut features = Vec::new();
    if observability.sentry.is_some() {
        features.push("obs-sentry".to_owned());
    }
    if observability.otel.is_some() {
        features.push("obs-otel".to_owned());
    }
    features
}

/// `[workspace.package]` of the workspace generated around a plugin crate.
/// Plugin crates inherit these fields (`license.workspace = true`, ...) and
/// Cargo rejects an inherited field the root leaves undefined, so they cannot
/// be dropped. Must equal the root
/// `Cargo.toml` `[workspace.package]` (checked by a unit test), as the
/// tooling ships prebuilt and cannot read the repository at runtime.
const WORKSPACE_PACKAGE: &[(&str, &str)] = &[
    ("version", r#""1.0.0""#),
    ("publish", "false"),
    ("edition", r#""2021""#),
    ("rust-version", r#""1.95""#),
    ("license", r#""Apache-2.0""#),
    ("repository", r#""https://github.com/kvnpetit/tynd""#),
    ("homepage", r#""https://kvnpetit.github.io/tynd/""#),
    (
        "documentation",
        r#""https://kvnpetit.github.io/tynd/docs/v1.0/""#,
    ),
    (
        "authors",
        r#"["Kevin Petit <74395438+kvnpetit@users.noreply.github.com>"]"#,
    ),
    (
        "keywords",
        r#"["desktop", "webview", "typescript", "bun", "gui"]"#,
    ),
    ("categories", r#"["gui", "web-programming"]"#),
];

/// `[workspace.dependencies]` entries plugin crates may inherit with
/// `workspace = true`; only the ones a linked plugin inherits are written to
/// the plugin's generated workspace manifest (cargo warns about unused ones). Each must equal the root `Cargo.toml` entry of the
/// same name (checked by a unit test) so plugins build against the versions
/// they are developed and tested with.
const WORKSPACE_DEPENDENCIES: &[(&str, &str)] = &[
    ("serde", r#"{ version = "1.0.229", features = ["derive"] }"#),
    ("serde_json", r#""1.0.154""#),
    ("parking_lot", r#""0.13""#),
    ("base64", r#""0.23""#),
    ("bon", r#""3""#),
    (
        "nutype",
        r#"{ version = "0.8", features = ["regex", "serde"] }"#,
    ),
    ("portable-pty", r#""0.9""#),
    ("tracing", r#""0.1""#),
    ("dashmap", r#""6""#),
    ("bytemuck", r#"{ version = "1", default-features = false }"#),
    ("crossbeam-channel", r#""0.5""#),
    (
        "specta",
        r#"{ version = "2.0.0-rc.28", features = ["derive"] }"#,
    ),
];

/// Sorted `tynd-host` features: the extra (telemetry) ones, `debug-protocol`
/// in debug builds, one `api-<name>` per plugin-first API, and `plugins` when
/// any native plugin is linked. Core APIs are always compiled into the host and
/// have no feature.
fn host_cargo_features(
    apis: &BTreeSet<String>,
    has_plugins: bool,
    extra_host_features: &[String],
    release: bool,
) -> Vec<String> {
    let mut host_features = extra_host_features.to_vec();
    if !release {
        host_features.push("debug-protocol".to_owned());
    }
    host_features.extend(
        apis.iter()
            .filter(|api| !CORE_HOST_APIS.contains(&api.as_str()))
            .map(|api| format!("api-{api}")),
    );
    if has_plugins {
        host_features.push("plugins".to_owned());
    }
    host_features.sort();
    host_features.dedup();
    host_features
}

/// `[dependencies]` lines linking each plugin crate with its selected features.
fn plugin_dependency_lines(plugins: &[NativeBuildPlugin]) -> String {
    plugins
        .iter()
        .map(|plugin| {
            let feature_clause = if plugin.features.is_empty() {
                String::new()
            } else {
                format!(
                    ", default-features = false, features = {}",
                    toml_array(&plugin.features)
                )
            };
            format!(
                "{} = {{ path = {}{} }}",
                plugin.crate_name,
                toml_path(&plugin.crate_dir),
                feature_clause
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Name of the plugin SDK crate plugins depend on.
const PLUGIN_SDK_CRATE: &str = "tynd-plugin-sdk";

/// Names of the dependencies the plugin crates inherit from the workspace
/// (`name.workspace = true`), in every dependency table of every `Cargo.toml`
/// copied with a plugin: nested crates included, `target/` skipped like the
/// copy does.
fn inherited_workspace_dependencies(plugins: &[NativeBuildPlugin]) -> Result<BTreeSet<String>> {
    let mut inherited = BTreeSet::new();
    for plugin in plugins {
        // Cargo reports the missing crate when the build runs.
        if !plugin.native_dir.is_dir() {
            continue;
        }
        let mut files = Vec::new();
        collect_native_files(
            &plugin.native_dir,
            Path::new(""),
            &mut HashSet::new(),
            &mut files,
        )
        .with_context(|| format!("scan native plugin {}", plugin.package))?;
        for (relative, manifest_path) in files {
            if relative.rsplit('/').next() != Some("Cargo.toml") {
                continue;
            }
            let source = fs::read_to_string(&manifest_path)
                .with_context(|| format!("read {}", manifest_path.display()))?;
            // A fixture or template manifest that is not valid TOML inherits
            // nothing; Cargo reports it if the build actually uses it.
            let manifest: toml::Table = match toml::from_str(&source) {
                Ok(manifest) => manifest,
                Err(error) => {
                    tracing::debug!("skip unparsable {}: {error}", manifest_path.display());
                    continue;
                },
            };
            collect_inherited_dependencies(&manifest, &mut inherited);
            for target in manifest
                .get("target")
                .and_then(toml::Value::as_table)
                .into_iter()
                .flat_map(toml::Table::values)
                .filter_map(toml::Value::as_table)
            {
                collect_inherited_dependencies(target, &mut inherited);
            }
        }
    }
    Ok(inherited)
}

fn collect_inherited_dependencies(table: &toml::Table, inherited: &mut BTreeSet<String>) {
    for section in ["dependencies", "dev-dependencies", "build-dependencies"] {
        let Some(dependencies) = table.get(section).and_then(toml::Value::as_table) else {
            continue;
        };
        for (name, value) in dependencies {
            if value.get("workspace").and_then(toml::Value::as_bool) == Some(true) {
                inherited.insert(name.clone());
            }
        }
    }
}

/// The workspace manifest written next to a plugin crate in its cache copy
/// (the crate is its only declared member, in `crate/`).
///
/// A plugin crate inherits package fields, lints and dependencies from a
/// workspace (`version.workspace = true`, ...), and Cargo looks for that
/// workspace in the directories above the crate, which no longer include the
/// app's `.tynd/build`. The copy therefore carries its own workspace root,
/// with what the plugin inherits: the package fields, an empty lint table,
/// the plugin SDK at `plugin_sdk` and the shared dependency versions.
pub(super) fn render_plugin_workspace_manifest(
    plugin: &NativeBuildPlugin,
    plugin_sdk: &Path,
) -> Result<String> {
    let inherited = inherited_workspace_dependencies(std::slice::from_ref(plugin))?;
    let mut workspace_dependencies = Vec::new();
    if inherited.contains(PLUGIN_SDK_CRATE) {
        workspace_dependencies.push(format!(
            "{PLUGIN_SDK_CRATE} = {{ path = {} }}",
            toml_path(plugin_sdk)
        ));
    }
    workspace_dependencies.extend(
        WORKSPACE_DEPENDENCIES
            .iter()
            .filter(|(name, _)| inherited.contains(*name))
            .map(|(name, value)| format!("{name} = {value}")),
    );
    Ok(format!(
        r#"[workspace]
resolver = "2"
members = [{crate_dir}]

[workspace.package]
{workspace_package}

# Plugin crates declare `[lints] workspace = true`; an empty table satisfies
# that inheritance without imposing the framework's lint policy on the build.
[workspace.lints]

[workspace.dependencies]
{workspace_dependencies}
"#,
        crate_dir = toml_quote(PLUGIN_COPY_CRATE_DIR),
        workspace_package = toml_entries(WORKSPACE_PACKAGE),
        workspace_dependencies = workspace_dependencies.join("\n"),
    ))
}

/// The manifest of the generated app workspace (`.tynd/build/Cargo.toml`). The
/// Tynd crates and the plugin crates are path dependencies with a workspace
/// of their own, so the app (the root package) is the only member.
///
/// The copies of the editable plugins are inside this directory
/// (`plugins/<crate>/crate`), where cargo would make them members of this
/// workspace, and resolve what they inherit (`edition.workspace = true`, ...)
/// against it instead of the workspace generated around each copy. `exclude`
/// keeps them out; it has no effect under a path listed in `members`, so
/// there is no `members = ["."]` (the root package is always a member).
pub(super) fn render_custom_cargo_manifest(
    app_name: &str,
    bin_name: &str,
    sources: &RustSourcePaths,
    apis: &BTreeSet<String>,
    plugins: &[NativeBuildPlugin],
    extra_host_features: &[String],
    release: bool,
) -> String {
    let host_features =
        host_cargo_features(apis, !plugins.is_empty(), extra_host_features, release);
    let plugin_dependencies = plugin_dependency_lines(plugins);

    format!(
        r#"[workspace]
resolver = "2"
exclude = [{local_plugins_dir}]

[package]
name    = {app_name}
version = "0.0.0"
edition = "2021"

[[bin]]
name = {bin_name}
path = "src/main.rs"

[dependencies]
tynd-runtime = {{ path = {runtime_path}, default-features = false }}
tynd-host    = {{ path = {host_path}, default-features = false, features = {host_features} }}
{plugin_dependencies}

[profile.release]
opt-level     = 3
lto           = "fat"
codegen-units = 1
panic         = "unwind"
strip         = true
debug         = false
incremental   = false
"#,
        local_plugins_dir = toml_quote(LOCAL_PLUGINS_DIR),
        app_name = toml_quote(app_name),
        bin_name = toml_quote(bin_name),
        runtime_path = toml_path(&sources.runtime),
        host_path = toml_path(&sources.host),
        host_features = toml_array(&host_features),
    )
}

pub(super) fn render_custom_main(plugins: &[NativeBuildPlugin]) -> Result<String> {
    let mut output = String::from("fn main() {\n");
    for plugin in plugins {
        let permissions = plugin
            .permissions
            .iter()
            .map(|permission| {
                serde_json::to_string(permission).map(|quoted| format!("{quoted}.to_owned()"))
            })
            .collect::<serde_json::Result<Vec<_>>>()?
            .join(", ");
        write!(
            &mut output,
            "    assert!(\n        tynd_host::plugins::register_with_permissions(\n            {}::__tynd_plugin(),\n            Some(vec![{}]),\n        ),\n        \"native plugin registration failed: {{}}\",\n        stringify!({}),\n    );\n",
            crate_ident(&plugin.crate_name),
            permissions,
            crate_ident(&plugin.crate_name)
        )?;
    }
    output.push_str("    tynd_runtime::run();\n}\n");
    Ok(output)
}

fn toml_entries(entries: &[(&str, &str)]) -> String {
    entries
        .iter()
        .map(|(key, value)| format!("{key} = {value}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn toml_path(path: &Path) -> String {
    toml_quote(&crate::fsx::portable_path(&path.to_string_lossy()))
}

fn toml_quote(value: &str) -> String {
    serde_json::to_string(value).expect("serializing a Rust string cannot fail")
}

fn toml_array(values: &[String]) -> String {
    format!(
        "[{}]",
        values
            .iter()
            .map(|value| toml_quote(value))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

pub(super) fn write_generated_file(path: &Path, bytes: &[u8]) -> Result<()> {
    if fs::read(path).is_ok_and(|current| current == bytes) {
        return Ok(());
    }
    let parent = path.parent().context("generated file has no parent")?;
    fs::create_dir_all(parent)?;
    if path.exists() && fs::symlink_metadata(path)?.file_type().is_symlink() {
        bail!(
            "refusing to overwrite generated-file symlink: {}",
            path.display()
        );
    }
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(bytes)?;
    temporary.as_file().sync_all()?;
    if path.exists() {
        fs::remove_file(path)?;
    }
    temporary
        .persist(path)
        .map_err(|error| anyhow::Error::new(error.error))?;
    Ok(())
}

#[cfg(test)]
mod tests;
