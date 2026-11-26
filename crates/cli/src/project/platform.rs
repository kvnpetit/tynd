use std::{
    env, fmt,
    path::{Path, PathBuf},
};

use anyhow::{bail, Result};
use serde::Serialize;

use crate::fsx::absolute;

use super::shared::{is_file, sanitize_crate_name};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Platform {
    Windows,
    Macos,
    Linux,
}

impl Platform {
    pub(crate) const fn npm_name(self) -> &'static str {
        match self {
            Self::Windows => "win32",
            Self::Macos => "darwin",
            Self::Linux => "linux",
        }
    }
}

impl fmt::Display for Platform {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Windows => "windows",
            Self::Macos => "macos",
            Self::Linux => "linux",
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Architecture {
    X64,
    Arm64,
}

impl Architecture {
    pub(crate) const fn npm_name(self) -> &'static str {
        match self {
            Self::X64 => "x64",
            Self::Arm64 => "arm64",
        }
    }
}

impl fmt::Display for Architecture {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.npm_name())
    }
}

pub(crate) fn current_platform() -> Result<Platform> {
    platform_from(env::consts::OS)
}

pub(crate) fn platform_from(platform: &str) -> Result<Platform> {
    match platform {
        "windows" | "win32" => Ok(Platform::Windows),
        "macos" | "darwin" => Ok(Platform::Macos),
        "linux" => Ok(Platform::Linux),
        _ => bail!("unsupported Tynd platform: {platform}"),
    }
}

pub(crate) fn current_architecture() -> Result<Architecture> {
    architecture_from(env::consts::ARCH)
}

pub(crate) fn architecture_from(architecture: &str) -> Result<Architecture> {
    match architecture {
        "x86_64" | "x64" | "amd64" => Ok(Architecture::X64),
        "aarch64" | "arm64" => Ok(Architecture::Arm64),
        _ => bail!("unsupported Tynd architecture: {architecture}"),
    }
}

/// Path of the app host for one profile:
/// `<cwd>/.tynd/build/bin/<release|debug>/<app>`. Cargo builds into a target
/// directory shared across projects; a successful build copies its binary
/// here, the only place the tooling reads it from. The file may not exist yet.
pub(crate) fn host_binary_path(cwd: &Path, release: bool) -> Result<PathBuf> {
    let root = absolute(cwd)?;
    let name = root
        .file_name()
        .and_then(|name| name.to_str())
        .map_or_else(|| "app".to_owned(), sanitize_crate_name);
    let profile = if release { "release" } else { "debug" };
    Ok(root
        .join(".tynd/build/bin")
        .join(profile)
        .join(executable_name(&name)?))
}

/// The already compiled app host for one profile (`tynd build` compiles the
/// release one, `tynd dev` the debug one).
pub(crate) fn resolve_host_binary(cwd: &Path, release: bool) -> Result<PathBuf> {
    let binary = host_binary_path(cwd, release)?;
    if is_file(&binary) {
        return Ok(binary);
    }
    let command = if release { "tynd build" } else { "tynd dev" };
    bail!(
        "Tynd host not compiled yet: {} is missing; run `{command}` first",
        binary.display()
    )
}

/// The compiled app host of either profile, release first.
pub(crate) fn find_host_binary(cwd: &Path) -> Result<Option<PathBuf>> {
    for release in [true, false] {
        let binary = host_binary_path(cwd, release)?;
        if is_file(&binary) {
            return Ok(Some(binary));
        }
    }
    Ok(None)
}

pub(super) fn executable_name(base: &str) -> Result<String> {
    Ok(match current_platform()? {
        Platform::Windows => format!("{base}.exe"),
        Platform::Macos | Platform::Linux => base.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn host_binary_lives_in_the_project_build_directory() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("My App");
        fs::create_dir_all(&root).expect("create project");
        let name = executable_name(&sanitize_crate_name("My App")).expect("binary name");
        let release = host_binary_path(&root, true).expect("release path");
        let debug = host_binary_path(&root, false).expect("debug path");
        assert!(release.ends_with(format!(".tynd/build/bin/release/{name}")));
        assert!(debug.ends_with(format!(".tynd/build/bin/debug/{name}")));
    }

    #[test]
    fn resolves_only_the_compiled_host_of_the_requested_profile() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("app");
        fs::create_dir_all(&root).expect("create project");
        assert_eq!(find_host_binary(&root).expect("lookup"), None);
        let error = resolve_host_binary(&root, true)
            .expect_err("not built")
            .to_string();
        assert!(error.contains("tynd build"), "{error}");

        let debug = host_binary_path(&root, false).expect("debug path");
        fs::create_dir_all(debug.parent().expect("profile dir")).expect("create profile dir");
        fs::write(&debug, "").expect("write host");
        assert_eq!(
            resolve_host_binary(&root, false).expect("debug host"),
            debug
        );
        assert!(resolve_host_binary(&root, true).is_err());
        assert_eq!(find_host_binary(&root).expect("lookup"), Some(debug));

        let release = host_binary_path(&root, true).expect("release path");
        fs::create_dir_all(release.parent().expect("profile dir")).expect("create profile dir");
        fs::write(&release, "").expect("write host");
        assert_eq!(find_host_binary(&root).expect("lookup"), Some(release));
    }
}
