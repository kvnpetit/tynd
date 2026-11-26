//! Locates the Tynd executables that ship together in one `bin/`.
//!
//! One order everywhere, however the running binary was launched: the
//! documented environment override, then the sibling of the running
//! executable (all binaries of a release ship side by side, so their versions
//! always match), then, in debug builds only, the cargo target directory that
//! test binaries run from. Nothing walks `node_modules` or `PATH`.

use std::{
    env,
    ffi::OsString,
    path::{Path, PathBuf},
};

use anyhow::{bail, Result};

use super::platform::executable_name;
use super::shared::is_file;

/// A Tynd executable resolved by [`resolve_binary`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TyndBinary {
    /// The project CLI, which `tynd-mcp` spawns for `app.launch`.
    Tynd,
    /// The packaging executable `tynd build --bundle` calls.
    Bundler,
}

impl TyndBinary {
    const fn base_name(self) -> &'static str {
        match self {
            Self::Tynd => "tynd",
            Self::Bundler => "tynd-bundler",
        }
    }

    /// Environment variable that overrides the lookup with an absolute path.
    pub(crate) const fn override_variable(self) -> &'static str {
        match self {
            Self::Tynd => "TYND_BIN",
            Self::Bundler => "TYND_BUNDLER_BIN",
        }
    }
}

/// Path of `binary`: its override variable, else the sibling of the running
/// executable, else (debug builds) the cargo target directory.
pub(crate) fn resolve_binary(binary: TyndBinary) -> Result<PathBuf> {
    let executable = env::current_exe().ok();
    resolve_from(
        binary,
        env::var_os(binary.override_variable()),
        executable.as_deref(),
    )
}

fn resolve_from(
    binary: TyndBinary,
    configured: Option<OsString>,
    executable: Option<&Path>,
) -> Result<PathBuf> {
    let variable = binary.override_variable();
    let file_name = executable_name(binary.base_name())?;
    if let Some(configured) = configured {
        let path = PathBuf::from(configured);
        validate_override(&path, variable)?;
        return Ok(path);
    }
    if let Some(found) = executable.and_then(|executable| bundled_binary(executable, &file_name)) {
        return Ok(found);
    }
    bail!(
        "{file_name} not found next to {}; reinstall @tynd/cli or set {variable} to its absolute path",
        executable.map_or_else(|| "the running executable".to_owned(), |path| path.display().to_string())
    )
}

/// `file_name` next to `executable`, or (debug builds) in the cargo profile
/// directory above `deps/`, where `cargo test` runs its binaries.
fn bundled_binary(executable: &Path, file_name: &str) -> Option<PathBuf> {
    let directory = executable.parent()?;
    let sibling = directory.join(file_name);
    if is_file(&sibling) {
        return Some(sibling);
    }
    cargo_profile_binary(directory, file_name)
}

#[cfg(debug_assertions)]
fn cargo_profile_binary(directory: &Path, file_name: &str) -> Option<PathBuf> {
    if directory.file_name()? != "deps" {
        return None;
    }
    let candidate = directory.parent()?.join(file_name);
    is_file(&candidate).then_some(candidate)
}

#[cfg(not(debug_assertions))]
fn cargo_profile_binary(_directory: &Path, _file_name: &str) -> Option<PathBuf> {
    None
}

fn validate_override(path: &Path, variable: &str) -> Result<()> {
    if !path.is_absolute() {
        bail!("{variable} must be an absolute path: {}", path.display());
    }
    #[cfg(windows)]
    if !path
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("exe"))
    {
        bail!("{variable} must end in .exe on Windows: {}", path.display());
    }
    if !is_file(path) {
        bail!("{variable} does not exist: {}", path.display());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    fn bundler_name() -> String {
        executable_name("tynd-bundler").expect("binary name")
    }

    #[test]
    fn the_override_wins_over_the_sibling() {
        let dir = tempfile::tempdir().expect("tempdir");
        let executable = dir.path().join(executable_name("tynd").expect("name"));
        fs::write(dir.path().join(bundler_name()), "").expect("write sibling");
        let configured = dir.path().join("custom").join(bundler_name());
        fs::create_dir_all(configured.parent().expect("parent")).expect("mkdir");
        fs::write(&configured, "").expect("write override");

        let resolved = resolve_from(
            TyndBinary::Bundler,
            Some(configured.clone().into_os_string()),
            Some(&executable),
        )
        .expect("override");
        assert_eq!(resolved, configured);
    }

    #[test]
    fn the_override_must_be_an_existing_absolute_path() {
        let relative = resolve_from(
            TyndBinary::Tynd,
            Some(OsString::from(executable_name("tynd").expect("name"))),
            None,
        )
        .expect_err("relative override")
        .to_string();
        assert!(
            relative.contains("TYND_BIN must be an absolute path"),
            "{relative}"
        );

        let dir = tempfile::tempdir().expect("tempdir");
        let missing = dir.path().join(bundler_name());
        let error = resolve_from(TyndBinary::Bundler, Some(missing.into_os_string()), None)
            .expect_err("missing override")
            .to_string();
        assert!(error.contains("TYND_BUNDLER_BIN does not exist"), "{error}");
    }

    #[test]
    fn the_sibling_of_the_running_executable_is_used() {
        let dir = tempfile::tempdir().expect("tempdir");
        let executable = dir.path().join(executable_name("tynd-mcp").expect("name"));
        let tynd = dir.path().join(executable_name("tynd").expect("name"));
        let error = resolve_from(TyndBinary::Tynd, None, Some(&executable))
            .expect_err("no sibling yet")
            .to_string();
        assert!(error.contains("set TYND_BIN"), "{error}");

        fs::write(&tynd, "").expect("write sibling");
        assert_eq!(
            resolve_from(TyndBinary::Tynd, None, Some(&executable)).expect("sibling"),
            tynd
        );
    }

    #[test]
    fn a_project_node_modules_copy_is_never_used() {
        let dir = tempfile::tempdir().expect("tempdir");
        let installed = dir.path().join("global");
        fs::create_dir_all(&installed).expect("mkdir");
        let executable = installed.join(executable_name("tynd").expect("name"));
        let stale = dir
            .path()
            .join("app/node_modules/@tynd/tooling-x/bin")
            .join(bundler_name());
        fs::create_dir_all(stale.parent().expect("parent")).expect("mkdir");
        fs::write(&stale, "").expect("write stale bundler");
        assert!(resolve_from(TyndBinary::Bundler, None, Some(&executable)).is_err());
    }

    #[test]
    fn debug_builds_find_the_cargo_profile_binary_above_deps() {
        let dir = tempfile::tempdir().expect("tempdir");
        let deps = dir.path().join("debug").join("deps");
        fs::create_dir_all(&deps).expect("mkdir");
        let test_binary = deps.join("tynd_cli-0123.exe");
        let bundler = dir.path().join("debug").join(bundler_name());
        fs::write(&bundler, "").expect("write bundler");
        let resolved = resolve_from(TyndBinary::Bundler, None, Some(&test_binary));
        if cfg!(debug_assertions) {
            assert_eq!(resolved.expect("cargo profile binary"), bundler);
        } else {
            assert!(resolved.is_err());
        }
    }
}
