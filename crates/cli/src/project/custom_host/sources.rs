//! The Rust sources a host build compiles: a Tynd checkout, a linked
//! package, or a host cache copy of an installed `@tynd/host-src`.

use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{bail, Context, Result};
use serde_json::Value;

use crate::project::{
    discovery::resolve_package_dir,
    host_cache::{host_sources_copy_dir, install_cache_copy, short_key},
    shared::walk_up,
};

use super::{
    fingerprint::{hash_field, hash_files},
    plugins::read_package_manifest,
};

/// The Rust sources one host build compiles.
#[derive(Clone, Debug)]
pub(super) struct HostSources {
    /// Directory cargo reads: the host cache copy of an installed
    /// `@tynd/host-src`, or editable sources (a Tynd checkout, a linked
    /// package) in place.
    pub(super) root: PathBuf,
    /// Version of the installed `@tynd/host-src`; `None` for editable sources.
    pub(super) version: Option<String>,
}

#[derive(Clone, Debug)]
pub(super) struct RustSourcePaths {
    pub(super) runtime: PathBuf,
    pub(super) host: PathBuf,
    pub(super) plugin_sdk: PathBuf,
}

/// The sources a host build started in `cwd` compiles; see
/// [`cached_host_sources`].
pub(super) fn resolve_host_sources(cwd: &Path, cache_root: &Path) -> Result<HostSources> {
    cached_host_sources(cache_root, resolve_rust_source_root(cwd)?)
}

/// Cargo keys a path package by its absolute path, so the Tynd crates of an
/// installed `@tynd/host-src` (in each project's `node_modules`) would be
/// recompiled for every project. They are compiled instead from one copy of
/// the package's `native` tree in the host cache, at
/// `<cache_root>/host/src/<key>`, created on first use. The key covers the
/// package name and version and the path and content of every file
/// (`Cargo.lock` included), so a copy can never be stale: different content is
/// a different directory.
///
/// Editable sources (a Tynd checkout, a linked package) are used in place:
/// their path is already the same for every project, and they change without
/// their version changing.
pub(super) fn cached_host_sources(cache_root: &Path, source_root: PathBuf) -> Result<HostSources> {
    let Some(version) = installed_host_sources_version(&source_root) else {
        return Ok(HostSources {
            root: source_root,
            version: None,
        });
    };
    let files = source_tree_files(&source_root)
        .with_context(|| format!("scan {}", source_root.display()))?;
    let mut hasher = blake3::Hasher::new();
    hash_field(&mut hasher, "package", HOST_SOURCES_PACKAGE.as_bytes());
    hash_field(&mut hasher, "version", version.as_bytes());
    hash_files(&mut hasher, &files)?;
    let copy = host_sources_copy_dir(cache_root, &short_key(&hasher.finalize()));
    install_cache_copy(cache_root, &copy, |temporary| {
        for (relative, path) in &files {
            let output = temporary.join(relative);
            if let Some(parent) = output.parent() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("create {}", parent.display()))?;
            }
            fs::copy(path, &output).with_context(|| format!("copy {}", path.display()))?;
        }
        Ok(())
    })
    .with_context(|| format!("copy {HOST_SOURCES_PACKAGE} to the host build cache"))?;
    Ok(HostSources {
        root: copy,
        version: Some(version),
    })
}

/// Every file of the source tree `root` that a cache copy holds; see
/// [`collect_native_files`].
pub(super) fn source_tree_files(root: &Path) -> Result<Vec<(String, PathBuf)>> {
    let mut files = Vec::new();
    collect_native_files(root, Path::new(""), &mut HashSet::new(), &mut files)?;
    Ok(files)
}

/// Version of the installed `@tynd/host-src` package owning `source_root` (its
/// `native` directory). `None` for a Tynd checkout and for a package that does
/// not live under `node_modules` (linked sources are edited in place).
pub(super) fn installed_host_sources_version(source_root: &Path) -> Option<String> {
    let package_dir = fs::canonicalize(source_root).ok()?.parent()?.to_path_buf();
    if !package_dir
        .components()
        .any(|component| component.as_os_str() == "node_modules")
    {
        return None;
    }
    let manifest = read_package_manifest(&package_dir).ok()?;
    if manifest.get("name").and_then(Value::as_str) != Some(HOST_SOURCES_PACKAGE) {
        return None;
    }
    manifest
        .get("version")
        .and_then(Value::as_str)
        .map(str::to_owned)
}

/// Every file of a source tree (a plugin crate, the host sources), as
/// (`/`-separated relative path, real path) pairs in a stable order, without
/// `target` and `node_modules` directories. This is what `copy_native_tree`
/// copies from a plugin crate: symlinks are followed like the copy does, and
/// one that leaves the crate is rejected by the copy itself.
pub(super) fn collect_native_files(
    directory: &Path,
    relative: &Path,
    visiting: &mut HashSet<PathBuf>,
    files: &mut Vec<(String, PathBuf)>,
) -> Result<()> {
    let directory_real = fs::canonicalize(directory)?;
    if !visiting.insert(directory_real.clone()) {
        bail!("source tree contains a cyclic directory symlink");
    }
    let mut entries = fs::read_dir(&directory_real)?.collect::<std::io::Result<Vec<_>>>()?;
    entries.sort_by_key(fs::DirEntry::file_name);
    for entry in entries {
        let name = entry.file_name();
        if name == "target" || name == "node_modules" {
            continue;
        }
        let relative = relative.join(&name);
        let Ok(target) = fs::canonicalize(entry.path()) else {
            continue;
        };
        if target.is_dir() {
            collect_native_files(&target, &relative, visiting, files)?;
        } else if target.is_file() {
            files.push((relative.to_string_lossy().replace('\\', "/"), target));
        }
    }
    visiting.remove(&directory_real);
    Ok(())
}

fn resolve_rust_source_root(cwd: &Path) -> Result<PathBuf> {
    for directory in walk_up(cwd) {
        if directory.join("crates/host/Cargo.toml").is_file()
            && directory.join("crates/runtime/Cargo.toml").is_file()
        {
            return Ok(directory);
        }
    }
    if let Some(package_dir) = resolve_package_dir(HOST_SOURCES_PACKAGE, cwd)? {
        let native = package_dir.join("native");
        if native.join("Cargo.toml").is_file() && native.join("runtime/Cargo.toml").is_file() {
            return Ok(native);
        }
    }
    bail!(
        "the Tynd host is compiled from Rust sources that are not installed; run `{}` in {}",
        host_sources_install_command(cwd),
        cwd.display()
    )
}

/// npm package shipping the Rust sources the app host is compiled from.
pub(crate) const HOST_SOURCES_PACKAGE: &str = "@tynd/host-src";

/// Whether a host build can find Tynd Rust sources from `cwd`.
pub(crate) fn host_sources_available(cwd: &Path) -> bool {
    resolve_rust_source_root(cwd).is_ok()
}

/// `bun add` command installing the host sources at the version range the
/// app already uses for its other `@tynd/*` packages.
pub(crate) fn host_sources_install_command(cwd: &Path) -> String {
    format!("bun add -d {}", host_sources_spec(cwd))
}

/// `@tynd/host-src@<range>`, passed to `bun add` as a single argument.
pub(crate) fn host_sources_spec(cwd: &Path) -> String {
    let manifest = fs::read(cwd.join("package.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok());
    format!(
        "{HOST_SOURCES_PACKAGE}@{}",
        tynd_version_range(manifest.as_ref())
    )
}

/// Range of the first `@tynd/cli` or `@tynd/core` dependency in
/// `manifest` when it is a plain semver range (`1.2.3`, `^1.2.3`, `~1.2.3`),
/// falling back to the range matching this CLI. Anything else (`workspace:*`,
/// `file:`, URLs, compound ranges) is not reused.
fn tynd_version_range(manifest: Option<&Value>) -> String {
    let declared = manifest.and_then(|manifest| {
        ["@tynd/cli", "@tynd/core"]
            .into_iter()
            .flat_map(|package| {
                ["dependencies", "devDependencies"]
                    .into_iter()
                    .map(move |section| (section, package))
            })
            .find_map(|(section, package)| {
                manifest
                    .get(section)
                    .and_then(|deps| deps.get(package))
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|range| !range.is_empty())
            })
    });
    declared
        .filter(|range| is_plain_semver_range(range))
        .map_or_else(|| format!("^{}", crate::VERSION), ToOwned::to_owned)
}

fn is_plain_semver_range(range: &str) -> bool {
    let version = range
        .strip_prefix('^')
        .or_else(|| range.strip_prefix('~'))
        .unwrap_or(range);
    semver::Version::parse(version).is_ok()
}

pub(super) fn rust_source_paths(root: &Path) -> Result<RustSourcePaths> {
    let standalone_runtime = root.join("runtime");
    let sources = if standalone_runtime.join("Cargo.toml").is_file() {
        RustSourcePaths {
            runtime: standalone_runtime,
            host: root.to_path_buf(),
            plugin_sdk: root.join("vendor/tynd-plugin-sdk"),
        }
    } else {
        RustSourcePaths {
            runtime: root.join("crates/runtime"),
            host: root.join("crates/host"),
            plugin_sdk: root.join("crates/plugin-sdk"),
        }
    };
    if !sources.runtime.join("Cargo.toml").is_file() {
        bail!(
            "Tynd runtime sources are incomplete: {}",
            sources.runtime.display()
        );
    }
    if !sources.host.join("Cargo.toml").is_file() {
        bail!(
            "Tynd host sources are incomplete: {}",
            sources.host.display()
        );
    }
    Ok(sources)
}

#[cfg(test)]
mod tests;
