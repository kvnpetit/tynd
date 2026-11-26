//! Digests of the build inputs: the dev build fingerprint and the
//! `Cargo.lock` seeded from the host sources.

use std::{
    fmt::Write as _,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};

use super::{
    manifest::write_generated_file,
    plugins::NativeBuildPlugin,
    sources::{source_tree_files, HostSources},
};

/// File next to the dev host binary (`.tynd/build/bin/debug`) recording the
/// inputs it was built from.
pub(super) const FINGERPRINT_FILE: &str = ".tynd-host-fingerprint";

/// One length-prefixed, named field of a digest.
pub(super) fn hash_field(hasher: &mut blake3::Hasher, name: &str, value: &[u8]) {
    hasher.update(name.as_bytes());
    hasher.update(&(value.len() as u64).to_le_bytes());
    hasher.update(value);
}

/// The relative path and the content of every file of a source tree (see
/// [`source_tree_files`](super::sources::source_tree_files)).
pub(super) fn hash_files(hasher: &mut blake3::Hasher, files: &[(String, PathBuf)]) -> Result<()> {
    for (relative, path) in files {
        hash_field(hasher, "path", relative.as_bytes());
        let content = fs::read(path).with_context(|| format!("read {}", path.display()))?;
        hash_field(hasher, "content", &content);
    }
    Ok(())
}

/// Digest of everything a dev host build depends on, or `None` when the Rust
/// sources can change without their version changing (a Tynd checkout or a
/// linked `@tynd/host-src`), in which case cargo decides what is stale.
///
/// Covered: the tooling and `@tynd/host-src` versions, the generated manifest
/// (APIs, Cargo features, plugin crates and their features, and the paths of
/// the content-addressed source copies),
/// the generated `main.rs` (plugin permissions), the updater key pin compiled
/// into the host, the `@tynd/host-src` `Cargo.lock`, the compiler and its
/// flags (`compiler`, see [`compiler_fingerprint_input`]), and the content of
/// every plugin native crate.
pub(super) fn host_build_fingerprint(
    sources: &HostSources,
    cargo_manifest: &str,
    generated_main: &str,
    plugins: &[NativeBuildPlugin],
    updater_pin: Option<&str>,
    compiler: &str,
) -> Result<Option<String>> {
    let Some(sources_version) = &sources.version else {
        return Ok(None);
    };
    let lock_path = sources.root.join(CARGO_LOCK);
    let sources_lock = match fs::read(&lock_path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(error) => {
            return Err(error).with_context(|| format!("read {}", lock_path.display()));
        },
    };
    let mut hasher = blake3::Hasher::new();
    let mut field = |name: &str, value: &[u8]| hash_field(&mut hasher, name, value);
    field("tooling", crate::VERSION.as_bytes());
    field("host-src", sources_version.as_bytes());
    field("manifest", cargo_manifest.as_bytes());
    field("main", generated_main.as_bytes());
    field("updater-pin", updater_pin.unwrap_or_default().as_bytes());
    field("host-src-lock", &sources_lock);
    field("compiler", compiler.as_bytes());
    for plugin in plugins {
        let files = source_tree_files(&plugin.native_dir)
            .with_context(|| format!("fingerprint native plugin {}", plugin.package))?;
        hash_field(&mut hasher, "plugin", plugin.crate_name.as_bytes());
        hash_files(&mut hasher, &files)?;
    }
    Ok(Some(hasher.finalize().to_hex().to_string()))
}

/// Compiler identity for the dev fingerprint: the full `rustc --version` line
/// plus the flag variables cargo passes to rustc, which change the binary
/// without changing any source.
pub(super) fn compiler_fingerprint_input(rustc_version: &str) -> String {
    let mut input = rustc_version.to_owned();
    for variable in RUSTFLAGS_VARIABLES {
        let value = std::env::var_os(variable).unwrap_or_default();
        let _ = write!(
            input,
            "
{variable}={}",
            value.to_string_lossy()
        );
    }
    input
}

/// Environment variables cargo turns into rustc flags.
const RUSTFLAGS_VARIABLES: [&str; 2] = ["RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS"];

const CARGO_LOCK: &str = "Cargo.lock";

/// File in `.tynd/build` recording which `@tynd/host-src` lockfile seeded the
/// app lockfile.
const LOCK_SEED_FILE: &str = ".tynd-lock-seed";

/// Seed `.tynd/build/Cargo.lock` from the host sources' lockfile so the app
/// host builds with the dependency set Tynd's CI tested. The lockfile is
/// copied when absent and replaced when the sources' lockfile changed (a new
/// `@tynd/host-src`); otherwise cargo's own updates (app and plugin
/// dependencies) are kept. Sources without a lockfile leave cargo to resolve.
pub(super) fn seed_cargo_lock(build_dir: &Path, source_root: &Path) -> Result<()> {
    let source_lock = source_root.join(CARGO_LOCK);
    let content = match fs::read(&source_lock) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(error).with_context(|| format!("read {}", source_lock.display()));
        },
    };
    let seed = blake3::hash(&content).to_hex().to_string();
    let lock = build_dir.join(CARGO_LOCK);
    let seed_path = build_dir.join(LOCK_SEED_FILE);
    if lock.is_file() && fs::read_to_string(&seed_path).is_ok_and(|stored| stored == seed) {
        return Ok(());
    }
    write_generated_file(&lock, &content)?;
    write_generated_file(&seed_path, seed.as_bytes())
}

#[cfg(test)]
mod tests;
