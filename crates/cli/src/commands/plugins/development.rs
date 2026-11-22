use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use crate::fsx::{absolute, atomic_write, copy_tree, portable_path, read_json, write_json_file};
use crate::output::{display_path, step, success};

use super::{shared::resolve_plugin_input, GenTypesRequest, PluginDevRequest};

const PLUGIN_DEV_MARKER: &str = ".tynd-plugin-dev";

pub(in crate::commands) fn plugin_dev(request: &PluginDevRequest) -> Result<()> {
    let plugin = resolve_plugin_input(&request.cwd, &request.plugin)?;
    let safe = plugin
        .name
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character
            } else {
                '-'
            }
        })
        .collect::<String>();
    let scratch_input = request.scratch.clone().unwrap_or_else(|| {
        dirs::home_dir()
            .unwrap_or_else(|| request.cwd.clone())
            .join(".tynd")
            .join("plugin-dev")
            .join(safe)
    });
    let scratch = safe_plugin_dev_scratch(&request.cwd, &scratch_input)?;
    let marker = scratch.join(PLUGIN_DEV_MARKER);
    if scratch.exists() {
        let metadata = fs::symlink_metadata(&scratch)?;
        if is_link_or_reparse(&metadata) || !metadata.is_dir() {
            bail!(
                "plugin scratch must be a real directory, not a link or file: {}",
                scratch.display()
            );
        }
        let empty = fs::read_dir(&scratch)?.next().is_none();
        if !marker.is_file() && !empty {
            bail!(
                "refusing to modify unmanaged scratch directory {}; choose an empty directory",
                scratch.display()
            );
        }
        if request.force {
            fs::remove_dir_all(&scratch)?;
        }
    }
    fs::create_dir_all(scratch.join("backend"))?;
    fs::create_dir_all(scratch.join("frontend"))?;
    atomic_write(
        &marker,
        format!("managed by tynd plugin dev for {}\n", plugin.name),
    )?;
    let plugin_manifest = read_json(&plugin.root.join("package.json"))?;
    write_json_file(
        &scratch.join("package.json"),
        &scratch_package_json(&plugin.name, &plugin.root, &plugin_manifest),
    )?;
    atomic_write(
        &scratch.join("tynd.config.ts"),
        format!(
            "import type {{ TyndConfig }} from \"@tynd/cli\"\nexport default {{ backend: \"backend/main.ts\", frontendDir: \"frontend\", plugins: [{}] }} satisfies TyndConfig\n",
            serde_json::to_string(&plugin.name)?
        ),
    )?;
    atomic_write(
        &scratch.join("backend/main.ts"),
        "import { app } from \"@tynd/core\"\napp.start({ frontendDir: import.meta.dir + \"/../frontend\" })\n",
    )?;
    atomic_write(
        &scratch.join("frontend/index.html"),
        "<!doctype html><meta charset=\"utf-8\"><title>Tynd plugin dev</title><h1>Tynd plugin dev</h1>\n",
    )?;
    let install = Command::new("bun")
        .arg("install")
        .current_dir(&scratch)
        .status()?;
    if !install.success() {
        bail!("bun install failed with {install}");
    }
    let current = std::env::current_exe()?;
    let status = Command::new(current)
        .arg(if request.watch { "dev" } else { "build" })
        .arg("--cwd")
        .arg(&scratch)
        .current_dir(&scratch)
        .env_clear()
        .envs(crate::process::sanitized_environment())
        .status()?;
    if !status.success() {
        bail!("plugin dev app exited with {status}");
    }
    Ok(())
}

/// `package.json` of the plugin dev scratch app: the plugin linked from its
/// directory, plus the Tynd packages it needs.
fn scratch_package_json(plugin_name: &str, plugin_root: &Path, plugin_manifest: &Value) -> Value {
    let tynd = |package: &str| {
        Value::String(tynd_package_specifier(
            plugin_manifest,
            plugin_root,
            package,
        ))
    };
    let mut dependencies = serde_json::Map::new();
    dependencies.insert(
        plugin_name.to_owned(),
        Value::String(format!(
            "link:{}",
            portable_path(&plugin_root.to_string_lossy())
        )),
    );
    dependencies.insert("@tynd/core".into(), tynd("@tynd/core"));
    let mut package = json!({
        "private": true,
        "type": "module",
        "dependencies": dependencies,
        "devDependencies": {
            "@tynd/cli": tynd("@tynd/cli"),
            "@tynd/host-src": tynd("@tynd/host-src")
        }
    });
    // The plugin's `overrides` / `resolutions` pin transitive packages too
    // (e.g. an unpublished `@tynd/tooling` that `@tynd/cli` depends on): the
    // scratch app needs the same, with local paths made absolute.
    for field in ["overrides", "resolutions"] {
        if let (Some(value), Some(object)) = (plugin_manifest.get(field), package.as_object_mut()) {
            object.insert(
                field.to_owned(),
                resolve_local_specifiers(value, plugin_root),
            );
        }
    }
    package
}

/// Copy of an `overrides` / `resolutions` value with every `file:` / `link:`
/// specifier resolved against `plugin_root` (npm `overrides` nest objects).
fn resolve_local_specifiers(value: &Value, plugin_root: &Path) -> Value {
    match value {
        Value::String(specifier) => Value::String(
            local_specifier(specifier, plugin_root).unwrap_or_else(|| specifier.clone()),
        ),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, value)| (key.clone(), resolve_local_specifiers(value, plugin_root)))
                .collect(),
        ),
        other => other.clone(),
    }
}

/// `file:` / `link:` `specifier` with its path made absolute against
/// `plugin_root`; `None` for any other protocol.
fn local_specifier(specifier: &str, plugin_root: &Path) -> Option<String> {
    ["file:", "link:"].into_iter().find_map(|protocol| {
        let path = Path::new(specifier.strip_prefix(protocol)?);
        let resolved = if path.is_absolute() {
            path.to_path_buf()
        } else {
            plugin_root.join(path)
        };
        let resolved = dunce::canonicalize(&resolved).unwrap_or(resolved);
        Some(format!(
            "{protocol}{}",
            portable_path(&resolved.to_string_lossy())
        ))
    })
}

/// Specifier of the Tynd `package` in the scratch app. A plugin developed
/// against local or pinned Tynd packages (`file:`, `link:`, `workspace:` or an
/// exact version in its `devDependencies` or `peerDependencies`) gets the same
/// ones, so unpublished packages work; anything else gets this CLI's range.
fn tynd_package_specifier(plugin_manifest: &Value, plugin_root: &Path, package: &str) -> String {
    let declared = ["devDependencies", "peerDependencies"]
        .iter()
        .find_map(|field| plugin_manifest.get(field)?.get(package)?.as_str());
    if let Some(specifier) = declared {
        if let Some(local) = local_specifier(specifier, plugin_root) {
            return local;
        }
        if specifier.starts_with("workspace:") {
            // The scratch app is outside the plugin's workspace: point at the
            // package the workspace installed for the plugin.
            if let Ok(Some(installed)) = crate::project::resolve_package_dir(package, plugin_root) {
                let installed = dunce::canonicalize(&installed).unwrap_or(installed);
                return format!("file:{}", portable_path(&installed.to_string_lossy()));
            }
        } else if semver::Version::parse(specifier.trim_start_matches('=')).is_ok() {
            return specifier.to_owned();
        }
    }
    format!("^{}", crate::VERSION)
}

fn safe_plugin_dev_scratch(cwd: &Path, scratch: &Path) -> Result<PathBuf> {
    if scratch.components().any(|component| {
        matches!(
            component,
            std::path::Component::CurDir | std::path::Component::ParentDir
        )
    }) {
        bail!("plugin scratch path must not contain dot or parent segments");
    }
    let scratch = absolute(scratch)?;
    if scratch.file_name().is_none() {
        bail!("plugin scratch cannot be a filesystem root");
    }
    let cwd = absolute(cwd)?;
    if scratch == cwd || cwd.starts_with(&scratch) {
        bail!("plugin scratch cannot be the project root or one of its ancestors");
    }
    if let Some(home) = dirs::home_dir().map(|path| absolute(&path)).transpose()? {
        if scratch == home || home.starts_with(&scratch) {
            bail!("plugin scratch cannot be the home directory or one of its ancestors");
        }
    }
    Ok(scratch)
}

fn is_link_or_reparse(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt as _;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
        metadata.file_type().is_symlink()
            || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

pub(in crate::commands) fn gen_types(request: &GenTypesRequest) -> Result<()> {
    let plugins = native_plugin_targets(&request.cwd, request.plugin.as_deref())?;
    if plugins.is_empty() {
        bail!("no local native plugins found");
    }
    for plugin in &plugins {
        generate_plugin_types(plugin, request.dry_run, request.check)?;
    }
    success(if request.check {
        "Plugin types are current"
    } else if request.dry_run {
        "Plugin type generation preview complete"
    } else {
        "Plugin types generated"
    });
    Ok(())
}

#[derive(Debug)]
struct NativePluginTarget {
    package_root: PathBuf,
    native_root: PathBuf,
    output: PathBuf,
    crate_name: String,
    bin: String,
}

struct TypegenCargoContext {
    _temporary: Option<tempfile::TempDir>,
    manifest: PathBuf,
    cwd: PathBuf,
}

fn generate_plugin_types(plugin: &NativePluginTarget, dry_run: bool, check: bool) -> Result<()> {
    let cargo = typegen_cargo_context(plugin)?;
    let output = Command::new("cargo")
        .args(["run", "--release", "--quiet", "--manifest-path"])
        .arg(&cargo.manifest)
        .args(["-p", &plugin.crate_name, "--bin", &plugin.bin])
        .current_dir(&cargo.cwd)
        .env_clear()
        .envs(crate::process::sanitized_environment())
        .output()
        .with_context(|| format!("run type generator in {}", plugin.native_root.display()))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        bail!(
            "type generation failed in {} with {}{}",
            plugin.native_root.display(),
            output.status,
            if stderr.is_empty() {
                String::new()
            } else {
                format!(": {stderr}")
            }
        );
    }
    let generated = String::from_utf8(output.stdout)
        .context("plugin type generator emitted non-UTF-8 output")?;
    if !generated.contains("export type CommandsCatalog") || !generated.contains("_commands") {
        bail!(
            "type generator {} emitted no Tynd command catalog",
            plugin.native_root.display()
        );
    }
    let existing = fs::read_to_string(&plugin.output).ok();
    if existing.as_deref() == Some(generated.as_str()) {
        step(format!("unchanged {}", display_path(&plugin.output)));
        return Ok(());
    }
    if check {
        bail!(
            "generated plugin types are stale: {}",
            plugin.output.display()
        );
    }
    if dry_run {
        step(format!("would update {}", display_path(&plugin.output)));
        return Ok(());
    }
    atomic_write(&plugin.output, generated)?;
    success(format!("Types -> {}", display_path(&plugin.output)));
    Ok(())
}

fn typegen_cargo_context(plugin: &NativePluginTarget) -> Result<TypegenCargoContext> {
    let manifest = plugin.native_root.join("Cargo.toml");
    let metadata = Command::new("cargo")
        .args([
            "metadata",
            "--no-deps",
            "--format-version",
            "1",
            "--manifest-path",
        ])
        .arg(&manifest)
        .current_dir(&plugin.native_root)
        .env_clear()
        .envs(crate::process::sanitized_environment())
        .output()
        .context("inspect native plugin Cargo workspace")?;
    if metadata.status.success() {
        return Ok(TypegenCargoContext {
            _temporary: None,
            manifest,
            cwd: plugin.native_root.clone(),
        });
    }

    let manifest_source = fs::read_to_string(&manifest)?;
    if !manifest_source.contains("workspace = true") {
        let stderr = String::from_utf8_lossy(&metadata.stderr).trim().to_owned();
        bail!(
            "cargo metadata failed for {}{}",
            manifest.display(),
            if stderr.is_empty() {
                String::new()
            } else {
                format!(": {stderr}")
            }
        );
    }
    let sdk_package =
        crate::project::resolve_package_dir("@tynd/plugin-sdk", &plugin.package_root)?
            .context("native type generation requires @tynd/plugin-sdk to be installed")?;
    let sdk = fs::canonicalize(sdk_package.join("native"))
        .context("installed @tynd/plugin-sdk has no native Rust SDK")?;
    let temporary = tempfile::Builder::new()
        .prefix("tynd-plugin-typegen-")
        .tempdir()?;
    let copied_plugin = temporary.path().join("plugin");
    copy_tree(&plugin.native_root, &copied_plugin)?;
    let workspace_manifest = temporary.path().join("Cargo.toml");
    atomic_write(&workspace_manifest, typegen_workspace_manifest(&sdk)?)?;
    Ok(TypegenCargoContext {
        manifest: workspace_manifest,
        cwd: temporary.path().to_path_buf(),
        _temporary: Some(temporary),
    })
}

/// Manifest of the temporary workspace that type generation builds a plugin
/// crate in when the crate inherits from a workspace it is not part of.
fn typegen_workspace_manifest(sdk: &Path) -> Result<String> {
    let sdk_path = portable_path(&sdk.to_string_lossy());
    Ok(format!(
        "[workspace]\nresolver = \"2\"\nmembers = [\"plugin\"]\n\n[workspace.package]\nversion = \"0.1.0\"\npublish = false\nedition = \"2021\"\nrust-version = \"1.95\"\nlicense = \"Apache-2.0\"\nrepository = \"https://github.com/tynd-apps/tynd\"\nhomepage = \"https://tynd-apps.github.io/tynd/\"\ndocumentation = \"https://tynd-apps.github.io/tynd/docs/v1.0/\"\nauthors = []\nkeywords = []\ncategories = []\nreadme = \"README.md\"\n\n[workspace.dependencies]\ntynd-plugin-sdk = {{ path = {} }}\n",
        serde_json::to_string(&sdk_path)?
    ))
}

fn native_plugin_targets(cwd: &Path, selected: Option<&str>) -> Result<Vec<NativePluginTarget>> {
    let mut package_roots = BTreeSet::new();
    if let Some(selected) = selected {
        package_roots.insert(resolve_plugin_input(cwd, selected)?.root);
    } else {
        let cwd = absolute(cwd)?;
        if cwd.join("package.json").is_file() {
            package_roots.insert(fs::canonicalize(&cwd)?);
        }
        let plugins = cwd.join("plugins");
        if plugins.is_dir() {
            for entry in fs::read_dir(&plugins)? {
                let entry = entry?;
                if entry.file_type()?.is_dir() && entry.path().join("package.json").is_file() {
                    package_roots.insert(fs::canonicalize(entry.path())?);
                }
            }
        }
    }
    let mut targets = Vec::new();
    for package_root in package_roots {
        if let Some(target) = native_plugin_target(&package_root)? {
            targets.push(target);
        }
    }
    targets.sort_by(|left, right| left.native_root.cmp(&right.native_root));
    Ok(targets)
}

fn native_plugin_target(package_root: &Path) -> Result<Option<NativePluginTarget>> {
    let manifest_path = package_root.join("package.json");
    let manifest = read_json(&manifest_path)?;
    let Some(native_value) = manifest.pointer("/tynd/native") else {
        return Ok(None);
    };
    if native_value.is_null() || native_value == &Value::Bool(false) {
        return Ok(None);
    }
    let native = native_value.as_object().with_context(|| {
        format!(
            "{} has an invalid package.json::tynd.native block",
            manifest_path.display()
        )
    })?;
    let native_path = native
        .get("path")
        .and_then(Value::as_str)
        .unwrap_or("native");
    let package_root = fs::canonicalize(package_root)?;
    let native_root = resolve_native_root(&package_root, native_path)?;
    let output = generated_catalog_path(&package_root, &manifest)?;
    let crate_name = cargo_package_name(&native_root)?;
    let bin = gen_types_binary(&native_root)?;
    Ok(Some(NativePluginTarget {
        package_root,
        native_root,
        output,
        crate_name,
        bin,
    }))
}

/// Canonical native crate directory of a plugin; `native_path` must stay
/// inside the (canonical) `package_root` and hold a `Cargo.toml`.
fn resolve_native_root(package_root: &Path, native_path: &str) -> Result<PathBuf> {
    let relative = Path::new(native_path);
    if relative.is_absolute()
        || relative.components().any(|component| {
            matches!(
                component,
                std::path::Component::ParentDir
                    | std::path::Component::RootDir
                    | std::path::Component::Prefix(_)
            )
        })
    {
        bail!("native plugin path must stay inside its package: {native_path}");
    }
    let native_root = fs::canonicalize(package_root.join(relative)).with_context(|| {
        format!(
            "native plugin directory does not exist: {}",
            package_root.join(relative).display()
        )
    })?;
    if !native_root.starts_with(package_root) || !native_root.join("Cargo.toml").is_file() {
        bail!(
            "native plugin path is invalid or has no Cargo.toml: {}",
            native_root.display()
        );
    }
    Ok(native_root)
}

/// The plugin's single `*.gen.ts` catalog, or `<runtime name>.gen.ts` when
/// none exists yet.
fn generated_catalog_path(package_root: &Path, manifest: &Value) -> Result<PathBuf> {
    let mut generated = fs::read_dir(package_root)?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            entry
                .file_type()
                .ok()
                .filter(fs::FileType::is_file)
                .map(|_| entry.path())
        })
        .filter(|path| {
            path.file_name()
                .and_then(|value| value.to_str())
                .is_some_and(|name| name.ends_with(".gen.ts"))
        })
        .collect::<Vec<_>>();
    generated.sort();
    Ok(match generated.as_slice() {
        [path] => path.clone(),
        [] => {
            let package_name = manifest
                .get("name")
                .and_then(Value::as_str)
                .context("native plugin package has no name")?;
            let local_name = package_name.rsplit('/').next().unwrap_or(package_name);
            let runtime_name = local_name.strip_prefix("plugin-").unwrap_or(local_name);
            package_root.join(format!("{runtime_name}.gen.ts"))
        },
        _ => bail!(
            "{} has multiple *.gen.ts files; keep one canonical generated catalog",
            package_root.display()
        ),
    })
}

fn cargo_package_name(native_root: &Path) -> Result<String> {
    let manifest_path = native_root.join("Cargo.toml");
    let source = fs::read_to_string(&manifest_path)?;
    let manifest: toml::Value =
        toml::from_str(&source).with_context(|| format!("parse {}", manifest_path.display()))?;
    manifest
        .get("package")
        .and_then(toml::Value::as_table)
        .and_then(|package| package.get("name"))
        .and_then(toml::Value::as_str)
        .map(ToOwned::to_owned)
        .context("native plugin Cargo.toml has no package.name")
}

fn gen_types_binary(native_root: &Path) -> Result<String> {
    let manifest_path = native_root.join("Cargo.toml");
    let source = fs::read_to_string(&manifest_path)?;
    let manifest: toml::Value =
        toml::from_str(&source).with_context(|| format!("parse {}", manifest_path.display()))?;
    let mut candidates = manifest
        .get("bin")
        .and_then(toml::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(toml::Value::as_table)
        .filter_map(|bin| {
            let name = bin.get("name").and_then(toml::Value::as_str)?;
            let path = bin.get("path").and_then(toml::Value::as_str).unwrap_or("");
            (name.contains("gen-types") || path.replace('\\', "/").ends_with("/gen-types.rs"))
                .then(|| name.to_owned())
        })
        .collect::<Vec<_>>();
    if candidates.is_empty() && native_root.join("src/bin/gen-types.rs").is_file() {
        candidates.push("gen-types".into());
    }
    candidates.sort();
    candidates.dedup();
    match candidates.as_slice() {
        [name] => Ok(name.clone()),
        [] => bail!("{} has no gen-types binary", manifest_path.display()),
        _ => bail!(
            "{} has multiple gen-types binaries: {}",
            manifest_path.display(),
            candidates.join(", ")
        ),
    }
}

#[cfg(test)]
mod tests;
