//! The copies of the native plugin crates cargo compiles: content-addressed
//! in the host cache for installed plugins, synchronized into the build
//! directory for editable ones.

use std::{
    collections::{BTreeSet, HashSet},
    fs,
    path::{Path, PathBuf},
};

use anyhow::{bail, Context, Result};

use crate::project::host_cache::{install_cache_copy, plugin_copy_dir, short_key};

use super::{
    fingerprint::{hash_field, hash_files},
    manifest::{render_plugin_workspace_manifest, write_generated_file},
    plugins::NativeBuildPlugin,
    sources::source_tree_files,
};

/// Point every plugin at the copy cargo compiles: an installed plugin (inside
/// `node_modules`) at its copy in the host cache (see [`cached_plugin_crate`]),
/// an editable one (linked by `tynd plugin dev`, or local) at its copy in the
/// app's build directory (see [`local_plugin_crate`]). Copies in
/// `build_dir/plugins` no plugin uses any more are removed.
pub(super) fn cache_native_plugins(
    cache_root: &Path,
    build_dir: &Path,
    plugins: &mut [NativeBuildPlugin],
    plugin_sdk: &Path,
) -> Result<()> {
    let mut local = BTreeSet::new();
    for plugin in plugins.iter_mut() {
        plugin.crate_dir = if is_installed_plugin(&plugin.native_dir) {
            cached_plugin_crate(cache_root, plugin, plugin_sdk)
        } else {
            local.insert(plugin.crate_name.clone());
            local_plugin_crate(build_dir, plugin, plugin_sdk)
        }
        .with_context(|| format!("copy native plugin {}", plugin.package))?;
    }
    prune_local_plugin_copies(build_dir, &local);
    Ok(())
}

/// Whether the (canonical) plugin crate `native_dir` is an installed package,
/// whose content only changes with its version. A linked or local plugin
/// resolves outside `node_modules` and is edited in place.
fn is_installed_plugin(native_dir: &Path) -> bool {
    native_dir
        .components()
        .any(|component| component.as_os_str() == "node_modules")
}

/// Directory of `.tynd/build` holding the copies of the editable plugins.
pub(super) const LOCAL_PLUGINS_DIR: &str = "plugins";

/// The crate cargo compiles for an editable plugin: a copy at the stable path
/// `<build_dir>/plugins/<crate>`, laid out like a cache copy (the crate in
/// `crate/` next to the generated workspace manifest). A content-addressed
/// copy would be a new directory, so a full recompile, on every edit; this
/// one is synchronized instead: only changed files are rewritten, and files
/// removed from the plugin are removed from the copy, so cargo recompiles
/// only what an edit touched.
fn local_plugin_crate(
    build_dir: &Path,
    plugin: &NativeBuildPlugin,
    plugin_sdk: &Path,
) -> Result<PathBuf> {
    let workspace_manifest = render_plugin_workspace_manifest(plugin, plugin_sdk)?;
    let copy = build_dir.join(LOCAL_PLUGINS_DIR).join(&plugin.crate_name);
    fs::create_dir_all(&copy).with_context(|| format!("create {}", copy.display()))?;
    let build_real =
        fs::canonicalize(build_dir).with_context(|| format!("resolve {}", build_dir.display()))?;
    if !fs::canonicalize(&copy)?.starts_with(&build_real) {
        bail!("refusing plugin copy directory outside .tynd/build");
    }
    let crate_dir = copy.join(PLUGIN_COPY_CRATE_DIR);
    let source_real = fs::canonicalize(&plugin.native_dir)
        .with_context(|| format!("resolve {}", plugin.native_dir.display()))?;
    let mut files = Vec::new();
    for (relative, path) in source_tree_files(&source_real)? {
        if !path.starts_with(&source_real) {
            bail!("native plugin symlink escapes its crate directory: {relative}");
        }
        let output = relative
            .split('/')
            .fold(crate_dir.clone(), |path, name| path.join(name));
        files.push((output, path));
    }
    let manifest = copy.join("Cargo.toml");
    let wanted = WantedTree::new(
        &copy,
        files
            .iter()
            .map(|(output, _)| output.clone())
            .chain([manifest.clone()]),
    );
    // Prune first, so a case-only rename, a file turned directory (or the
    // reverse) or a planted symlink never blocks or redirects the writes.
    remove_unlisted_entries(&copy, &wanted)?;
    for (output, path) in files {
        let content = fs::read(&path).with_context(|| format!("read {}", path.display()))?;
        write_plugin_copy_file(&output, &content, &build_real)?;
    }
    write_plugin_copy_file(&manifest, workspace_manifest.as_bytes(), &build_real)?;
    Ok(crate_dir)
}

/// The files an editable plugin copy must hold, and the directories leading
/// to them, with their exact (case-sensitive) names.
struct WantedTree {
    files: HashSet<PathBuf>,
    directories: HashSet<PathBuf>,
}

impl WantedTree {
    fn new(root: &Path, files: impl IntoIterator<Item = PathBuf>) -> Self {
        let files: HashSet<PathBuf> = files.into_iter().collect();
        let mut directories = HashSet::new();
        for file in &files {
            let mut parent = file.parent();
            while let Some(directory) = parent {
                if directory == root || !directory.starts_with(root) {
                    break;
                }
                if !directories.insert(directory.to_path_buf()) {
                    break;
                }
                parent = directory.parent();
            }
        }
        Self { files, directories }
    }
}

/// Remove every entry under `directory` the copy must not hold: a symlink or
/// junction, a file or directory not wanted, one whose type differs from the
/// wanted one, or whose on-disk name differs from the wanted name (by case
/// only, on a case-insensitive file system); then the directories left
/// empty. Wanted entries are left untouched, so cargo sees their mtime.
fn remove_unlisted_entries(directory: &Path, wanted: &WantedTree) -> Result<()> {
    for entry in fs::read_dir(directory)? {
        let path = entry?.path();
        let file_type = fs::symlink_metadata(&path)?.file_type();
        if file_type.is_dir() && wanted.directories.contains(&path) {
            remove_unlisted_entries(&path, wanted)?;
            if fs::read_dir(&path)?.next().is_none() {
                fs::remove_dir(&path)?;
            }
        } else if file_type.is_dir() {
            fs::remove_dir_all(&path).with_context(|| format!("remove {}", path.display()))?;
        } else if file_type.is_symlink() || !wanted.files.contains(&path) {
            fs::remove_file(&path)
                .or_else(|_| fs::remove_dir(&path))
                .with_context(|| format!("remove {}", path.display()))?;
        }
    }
    Ok(())
}

/// Write `bytes` to `path` in an editable plugin copy, refusing a parent
/// directory that resolves outside `build_real` (the canonical build
/// directory).
fn write_plugin_copy_file(path: &Path, bytes: &[u8], build_real: &Path) -> Result<()> {
    let parent = path.parent().context("plugin copy file has no parent")?;
    fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    let parent_real =
        fs::canonicalize(parent).with_context(|| format!("resolve {}", parent.display()))?;
    if !parent_real.starts_with(build_real) {
        bail!(
            "refusing plugin copy file outside .tynd/build: {}",
            path.display()
        );
    }
    write_generated_file(path, bytes)
}

/// Remove the copies in `build_dir/plugins` of plugins other than `in_use`
/// (the crate names of the editable plugins of this build), and the directory
/// itself when no plugin uses it: earlier builds kept every plugin copy there.
/// Best effort, and only a directory that really is inside the build
/// directory.
fn prune_local_plugin_copies(build_dir: &Path, in_use: &BTreeSet<String>) {
    let plugins_dir = build_dir.join(LOCAL_PLUGINS_DIR);
    let inside_build_dir = fs::canonicalize(build_dir)
        .and_then(|build_real| Ok(fs::canonicalize(&plugins_dir)?.starts_with(build_real)))
        .unwrap_or(false);
    if !inside_build_dir {
        return;
    }
    if in_use.is_empty() {
        let _ = fs::remove_dir_all(&plugins_dir);
        return;
    }
    let Ok(entries) = fs::read_dir(&plugins_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let in_use = entry
            .file_name()
            .to_str()
            .is_some_and(|name| in_use.contains(name));
        if !in_use {
            let path = entry.path();
            let _ = fs::remove_dir_all(&path).or_else(|_| fs::remove_file(&path));
        }
    }
}

/// Directory of the plugin crate inside its cache copy, next to the generated
/// workspace manifest.
pub(super) const PLUGIN_COPY_CRATE_DIR: &str = "crate";

/// The crate cargo compiles for `plugin`: a copy in the host cache, created
/// on first use and shared by every project linking the same plugin content,
/// so cargo (which keys a path package by its absolute path) compiles it once.
///
/// The copy is `<cache_root>/host/plugins/<crate>-<key>`, holding the crate
/// in `crate/` and a generated workspace manifest (see
/// [`render_plugin_workspace_manifest`](super::manifest::render_plugin_workspace_manifest)).
/// The key covers both, so a copy can never be stale: different content is a
/// different directory.
fn cached_plugin_crate(
    cache_root: &Path,
    plugin: &NativeBuildPlugin,
    plugin_sdk: &Path,
) -> Result<PathBuf> {
    let workspace_manifest = render_plugin_workspace_manifest(plugin, plugin_sdk)?;
    let files = source_tree_files(&plugin.native_dir)?;
    let mut hasher = blake3::Hasher::new();
    hash_field(&mut hasher, "crate", plugin.crate_name.as_bytes());
    hash_field(&mut hasher, "workspace", workspace_manifest.as_bytes());
    hash_files(&mut hasher, &files)?;
    let copy = plugin_copy_dir(
        cache_root,
        &plugin.crate_name,
        &short_key(&hasher.finalize()),
    );
    install_cache_copy(cache_root, &copy, |temporary| {
        copy_native_tree(&plugin.native_dir, &temporary.join(PLUGIN_COPY_CRATE_DIR))?;
        fs::write(temporary.join("Cargo.toml"), &workspace_manifest)?;
        Ok(())
    })?;
    Ok(copy.join(PLUGIN_COPY_CRATE_DIR))
}

fn copy_native_tree(source: &Path, destination: &Path) -> Result<()> {
    let source_real =
        fs::canonicalize(source).with_context(|| format!("resolve {}", source.display()))?;
    let mut visiting = HashSet::new();
    copy_native_directory(&source_real, destination, &source_real, &mut visiting)
}

fn copy_native_directory(
    source: &Path,
    destination: &Path,
    allowed_root: &Path,
    visiting: &mut HashSet<PathBuf>,
) -> Result<()> {
    let source_real = fs::canonicalize(source)?;
    if !source_real.starts_with(allowed_root) {
        bail!("native plugin symlink escapes its crate directory");
    }
    if !visiting.insert(source_real.clone()) {
        bail!("native plugin contains a cyclic directory symlink");
    }
    fs::create_dir_all(destination)?;
    let mut entries = fs::read_dir(&source_real)?.collect::<std::io::Result<Vec<_>>>()?;
    entries.sort_by_key(fs::DirEntry::file_name);
    for entry in entries {
        let name = entry.file_name();
        if name == "target" || name == "node_modules" {
            continue;
        }
        copy_native_entry(
            &entry.path(),
            &destination.join(&name),
            allowed_root,
            visiting,
        )?;
    }
    visiting.remove(&source_real);
    Ok(())
}

/// Copy one directory entry. A symlink is followed only when its target stays
/// inside `allowed_root`; anything that is neither a file nor a directory is
/// skipped.
fn copy_native_entry(
    input: &Path,
    output: &Path,
    allowed_root: &Path,
    visiting: &mut HashSet<PathBuf>,
) -> Result<()> {
    let metadata = fs::symlink_metadata(input)?;
    if !metadata.file_type().is_symlink() {
        if metadata.is_dir() {
            copy_native_directory(input, output, allowed_root, visiting)?;
        } else if metadata.is_file() {
            fs::copy(input, output)?;
        }
        return Ok(());
    }
    let target = fs::canonicalize(input)?;
    if !target.starts_with(allowed_root) {
        bail!(
            "native plugin symlink escapes its crate directory: {}",
            input.display()
        );
    }
    if target.is_dir() {
        copy_native_directory(&target, output, allowed_root, visiting)?;
    } else if target.is_file() {
        fs::copy(&target, output)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
