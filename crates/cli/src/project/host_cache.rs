//! Shared cargo target directory for app host builds.
//!
//! Every app compiles its own host, but the dependency graph (wry, tao, ...)
//! is the same for one Tynd version and compiler. Building all apps into one
//! target directory per (host sources, compiler, host triple) lets cargo reuse
//! those artifacts across projects; cargo keys artifacts by feature set, so
//! apps with different APIs coexist, and its target-directory lock serializes
//! concurrent builds.
//!
//! Cargo keys a path package by its absolute path, so the Tynd crates and the
//! plugin crates are only reused when every project compiles them from the
//! same place: installed host sources and plugin crates are copied once into
//! content-addressed directories of this cache (`host/src/<key>`,
//! `host/plugins/<crate>-<key>`), which are never modified afterwards.

use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{bail, Context, Result};

/// Environment variable overriding the cache root (opt out of the OS user
/// cache directory, or isolate a CI job).
pub(crate) const CACHE_DIR_VARIABLE: &str = "TYND_CACHE_DIR";

/// Root of Tynd's user-level cache: `TYND_CACHE_DIR` when set, else the OS
/// user cache directory (`%LOCALAPPDATA%\tynd\cache`, `~/Library/Caches/tynd`,
/// `$XDG_CACHE_HOME/tynd` or `~/.cache/tynd`), made absolute with native
/// separators so the paths cargo and the linker see stay short and uniform.
pub(crate) fn cache_root() -> Result<PathBuf> {
    cache_root_from(std::env::var_os(CACHE_DIR_VARIABLE), dirs::cache_dir())
}

fn cache_root_from(
    override_dir: Option<OsString>,
    user_cache_dir: Option<PathBuf>,
) -> Result<PathBuf> {
    let root = if let Some(directory) = override_dir.filter(|value| !value.is_empty()) {
        PathBuf::from(directory)
    } else {
        let Some(base) = user_cache_dir else {
            bail!(
                "no user cache directory found; set {CACHE_DIR_VARIABLE} to a writable directory"
            );
        };
        // On Windows the base is `%LOCALAPPDATA%`, shared with application data.
        if cfg!(windows) {
            base.join("tynd").join("cache")
        } else {
            base.join("tynd")
        }
    };
    std::path::absolute(&root).with_context(|| format!("resolve {}", root.display()))
}

/// Directory holding every shared host target directory; what
/// `tynd clean --cache` removes.
pub(crate) fn host_cache_dir() -> Result<PathBuf> {
    Ok(host_cache_dir_in(&cache_root()?))
}

fn host_cache_dir_in(cache_root: &Path) -> PathBuf {
    cache_root.join("host")
}

/// Length of a cache key, in hex digits: short, because Windows build tools
/// (`link.exe`, MASM) fail on the deep paths cargo creates under it.
const HOST_CACHE_KEY_LEN: usize = 12;

/// Key of one shared target directory. `sources` identifies the Rust sources
/// (the `@tynd/host-src` version, or the path of editable sources),
/// `rustc_version` is the `rustc --version` line and `host_triple` the
/// compiler's host target.
pub(super) fn host_cache_key(sources: &str, rustc_version: &str, host_triple: &str) -> String {
    let mut hasher = blake3::Hasher::new();
    for field in [sources, rustc_version, host_triple] {
        hasher.update(&(field.len() as u64).to_le_bytes());
        hasher.update(field.as_bytes());
    }
    short_key(&hasher.finalize())
}

/// The first [`HOST_CACHE_KEY_LEN`] hex digits of `hash`: the name of a
/// content-addressed cache directory.
pub(super) fn short_key(hash: &blake3::Hash) -> String {
    hash.to_hex()[..HOST_CACHE_KEY_LEN].to_owned()
}

/// File marking a directory as Tynd's host cache; `tynd clean --cache`
/// refuses to remove a `host` directory without it, so a mistaken
/// `TYND_CACHE_DIR` cannot point it at unrelated data.
const HOST_CACHE_MARKER: &str = ".tynd-cache";

/// Cargo target directory for one cache key: `<cache_root>/host/<key>`.
pub(super) fn host_target_dir(cache_root: &Path, key: &str) -> PathBuf {
    host_cache_dir_in(cache_root).join(key)
}

/// [`host_target_dir`], after creating the host cache directory and its
/// marker file (see [`ensure_host_cache_dir`]).
pub(super) fn prepare_host_target_dir(cache_root: &Path, key: &str) -> Result<PathBuf> {
    ensure_host_cache_dir(cache_root)?;
    Ok(host_target_dir(cache_root, key))
}

/// Copy of the installed host sources with content key `key`:
/// `<cache_root>/host/src/<key>`.
pub(super) fn host_sources_copy_dir(cache_root: &Path, key: &str) -> PathBuf {
    host_cache_dir_in(cache_root).join("src").join(key)
}

/// Copy of the plugin crate `crate_name` with content key `key`:
/// `<cache_root>/host/plugins/<crate_name>-<key>`.
pub(super) fn plugin_copy_dir(cache_root: &Path, crate_name: &str, key: &str) -> PathBuf {
    host_cache_dir_in(cache_root)
        .join("plugins")
        .join(format!("{crate_name}-{key}"))
}

/// Make sure the content-addressed directory `destination` (somewhere under
/// the host cache of `cache_root`) exists. An existing directory is used as
/// is and never modified. Otherwise `fill` writes the content into a
/// temporary sibling directory, which is then renamed to `destination`, so
/// other processes see the complete copy or none; when another process wins
/// that race its copy is used.
pub(super) fn install_cache_copy(
    cache_root: &Path,
    destination: &Path,
    fill: impl FnOnce(&Path) -> Result<()>,
) -> Result<()> {
    if destination.is_dir() {
        return Ok(());
    }
    ensure_host_cache_dir(cache_root)?;
    let parent = destination
        .parent()
        .context("cache copy has no parent directory")?;
    fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    remove_stale_temporaries(parent, STALE_TEMPORARY_AGE);
    let temporary = tempfile::Builder::new()
        .prefix(".tmp-")
        .tempdir_in(parent)
        .with_context(|| format!("create a temporary directory in {}", parent.display()))?;
    fill(temporary.path())?;
    // Dropping `temporary` removes whatever is left at its path: nothing
    // after a successful rename, the redundant copy after a lost race.
    rename_cache_copy(temporary.path(), destination)
        .with_context(|| format!("move the cache copy to {}", destination.display()))
}

/// Age past which a `.tmp-*` directory of the cache is a copy left behind by
/// a crashed or killed process rather than one still being written.
const STALE_TEMPORARY_AGE: std::time::Duration = std::time::Duration::from_hours(1);

/// Remove the `.tmp-*` directories of `parent` last modified more than
/// `max_age` ago. Best effort: errors (a copy another process is removing,
/// an unreadable entry) are ignored.
fn remove_stale_temporaries(parent: &Path, max_age: std::time::Duration) {
    let Ok(entries) = fs::read_dir(parent) else {
        return;
    };
    for entry in entries.flatten() {
        let is_temporary = entry
            .file_name()
            .to_str()
            .is_some_and(|name| name.starts_with(".tmp-"));
        let is_stale = entry
            .metadata()
            .ok()
            .filter(fs::Metadata::is_dir)
            .and_then(|metadata| metadata.modified().ok())
            .and_then(|modified| modified.elapsed().ok())
            .is_some_and(|age| age >= max_age);
        if is_temporary && is_stale {
            let _ = fs::remove_dir_all(entry.path());
        }
    }
}

/// Attempts at renaming a finished copy into place, [`RENAME_RETRY_DELAY`]
/// apart: on Windows a scanner holding a freshly written file open makes the
/// rename fail for a moment.
const RENAME_ATTEMPTS: u32 = 5;
const RENAME_RETRY_DELAY: std::time::Duration = std::time::Duration::from_millis(200);

fn rename_cache_copy(temporary: &Path, destination: &Path) -> std::io::Result<()> {
    let mut attempt = 1;
    loop {
        let Err(error) = fs::rename(temporary, destination) else {
            return Ok(());
        };
        // Another process installed the same content first.
        if destination.is_dir() {
            return Ok(());
        }
        if attempt == RENAME_ATTEMPTS || error.kind() != std::io::ErrorKind::PermissionDenied {
            return Err(error);
        }
        attempt += 1;
        std::thread::sleep(RENAME_RETRY_DELAY);
    }
}

/// Create the host cache directory and its marker file. The marker is only
/// written into a `host` directory Tynd creates (absent or empty), never into
/// existing unrelated data.
fn ensure_host_cache_dir(cache_root: &Path) -> Result<()> {
    let host_cache_dir = host_cache_dir_in(cache_root);
    let marker = host_cache_dir.join(HOST_CACHE_MARKER);
    if !marker.is_file() {
        adopt_host_cache_dir(&host_cache_dir, &marker)?;
    }
    Ok(())
}

/// [`ensure_host_cache_dir`] once the marker was found missing: write it into
/// an absent or empty `host_cache_dir`. Another process may create the cache
/// meanwhile, so a directory whose marker has appeared since is valid too.
fn adopt_host_cache_dir(host_cache_dir: &Path, marker: &Path) -> Result<()> {
    let is_empty = match fs::read_dir(host_cache_dir) {
        Ok(mut entries) => entries.next().is_none(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => true,
        Err(error) => {
            return Err(error).with_context(|| format!("read {}", host_cache_dir.display()));
        },
    };
    if !is_empty {
        if marker.is_file() {
            return Ok(());
        }
        bail!(
            "refusing to use {} as the host build cache: it is not empty and has no \
             {HOST_CACHE_MARKER} marker, so Tynd did not create it; check \
             {CACHE_DIR_VARIABLE}",
            host_cache_dir.display()
        );
    }
    fs::create_dir_all(host_cache_dir)
        .with_context(|| format!("create {}", host_cache_dir.display()))?;
    fs::write(
        marker,
        "Tynd shared host build cache; `tynd clean --cache` removes this directory.\n",
    )
    .with_context(|| format!("write {}", marker.display()))?;
    Ok(())
}

/// Longest shared target directory path that keeps Windows builds under the
/// tool path limits (`link.exe` LNK1104, MASM A1009 in deep build scripts).
const MAX_WINDOWS_TARGET_DIR_LEN: usize = 80;

/// Warning to show when the shared target directory is long enough to break
/// Windows builds; `None` on other platforms or for a short enough path.
pub(super) fn long_target_dir_warning(target_dir: &Path) -> Option<String> {
    long_target_dir_warning_for(target_dir, cfg!(windows))
}

/// [`long_target_dir_warning`] for the target directory a build would use
/// under `host_cache_dir`, before its key is known.
pub(crate) fn long_host_cache_warning(host_cache_dir: &Path) -> Option<String> {
    long_target_dir_warning(&host_cache_dir.join("0".repeat(HOST_CACHE_KEY_LEN)))
}

fn long_target_dir_warning_for(target_dir: &Path, windows: bool) -> Option<String> {
    let length = target_dir.as_os_str().len();
    (windows && length > MAX_WINDOWS_TARGET_DIR_LEN).then(|| {
        format!(
            "the shared host build directory {} is {length} characters long; Windows build \
             tools can fail on paths this deep (LNK1104, A1009); set {CACHE_DIR_VARIABLE} to \
             a shorter directory such as C:\\tynd-cache",
            target_dir.display()
        )
    })
}

/// Remove the shared host cache. Returns whether there was one; with
/// `dry_run` nothing is removed. A directory without the marker Tynd writes
/// when it creates the cache is refused.
pub(crate) fn remove_host_cache(host_cache_dir: &Path, dry_run: bool) -> Result<bool> {
    if !host_cache_dir.exists() {
        return Ok(false);
    }
    if !host_cache_dir.join(HOST_CACHE_MARKER).is_file() {
        bail!(
            "refusing to remove {}: it has no {HOST_CACHE_MARKER} marker, so Tynd did not \
             create it; check {CACHE_DIR_VARIABLE} or remove the directory by hand",
            host_cache_dir.display()
        );
    }
    if !dry_run {
        fs::remove_dir_all(host_cache_dir)
            .with_context(|| format!("remove {}", host_cache_dir.display()))?;
    }
    Ok(true)
}

#[cfg(test)]
mod tests;
