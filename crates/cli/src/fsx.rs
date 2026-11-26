//! Filesystem helpers shared by every module.

use anyhow::{bail, Context, Result};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

pub(crate) fn absolute(path: &Path) -> Result<PathBuf> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(std::env::current_dir()?.join(path))
    }
}

/// Normalizes a path to forward slashes and drops the Windows verbatim prefix
/// that `fs::canonicalize` adds (`\\?\C:\x` becomes `C:/x`, `\\?\UNC\srv\share`
/// becomes `//srv/share`). Cargo rejects verbatim paths in a manifest and Bun
/// does not resolve them in an import.
pub(crate) fn portable_path(path: &str) -> String {
    let path = path.replace('\\', "/");
    if let Some(unc) = path.strip_prefix("//?/UNC/") {
        format!("//{unc}")
    } else if let Some(stripped) = path.strip_prefix("//?/") {
        stripped.to_owned()
    } else {
        path
    }
}

pub(crate) fn atomic_write(path: &Path, bytes: impl AsRef<[u8]>) -> Result<()> {
    let parent = path.parent().context("output path has no parent")?;
    fs::create_dir_all(parent)?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    temp.write_all(bytes.as_ref())?;
    temp.as_file().sync_all()?;
    temp.persist(path)
        .map_err(|error| anyhow::Error::new(error.error))?;
    Ok(())
}

pub(crate) fn copy_tree(source: &Path, destination: &Path) -> Result<()> {
    if !source.is_dir() {
        bail!("source directory not found: {}", source.display());
    }
    for entry in walkdir::WalkDir::new(source).follow_links(false) {
        let entry = entry?;
        let relative = entry.path().strip_prefix(source)?;
        let output = destination.join(relative);
        if entry.file_type().is_symlink() {
            bail!(
                "symbolic links are not accepted while copying {}",
                source.display()
            );
        }
        if entry.file_type().is_dir() {
            fs::create_dir_all(&output)?;
        } else if entry.file_type().is_file() {
            if let Some(parent) = output.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(entry.path(), &output).with_context(|| {
                format!("copy {} -> {}", entry.path().display(), output.display())
            })?;
        }
    }
    Ok(())
}

pub(crate) fn read_json(path: &Path) -> Result<serde_json::Value> {
    let bytes = fs::read(path).with_context(|| format!("read {}", path.display()))?;
    serde_json::from_slice(&bytes).with_context(|| format!("parse {}", path.display()))
}

pub(crate) fn write_json_file(path: &Path, value: &serde_json::Value) -> Result<()> {
    let mut bytes = serde_json::to_vec_pretty(value)?;
    bytes.push(b'\n');
    atomic_write(path, bytes)
}
