use std::path::Path;

use anyhow::{bail, Context, Result};
use semver::Version;

use crate::process::{find_executable, run_spec, CommandSpec};

/// Minimum Rust version that compiles the Tynd host: the root `Cargo.toml`
/// `[workspace.package] rust-version`, which this crate and `crates/host` both
/// inherit, captured at compile time as the tooling ships prebuilt and cannot
/// read the repository at runtime.
pub(crate) const HOST_MSRV: &str = env!("CARGO_PKG_RUST_VERSION");

/// Where a developer gets the Rust toolchain.
pub(crate) const RUST_INSTALL_URL: &str = "https://rustup.rs";

/// The Rust toolchain found on `PATH`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RustToolchain {
    pub rustc: Version,
    /// Full `rustc --version` line (includes the commit hash and date).
    pub version_output: String,
    /// The compiler's host target triple, which cargo builds the app host for.
    pub host: String,
}

/// Check that the Rust toolchain compiling the app host is usable: `cargo` and
/// `rustc` are on `PATH` and `rustc` is at least [`HOST_MSRV`]. `build_dir` is
/// the directory cargo runs in: probing from there makes rustup apply the same
/// `rust-toolchain.toml` override the build will use.
pub(crate) fn ensure_rust_toolchain(build_dir: &Path) -> Result<RustToolchain> {
    for tool in ["cargo", "rustc"] {
        if find_executable(tool).is_none() {
            bail!(
                "{tool} was not found on PATH; Tynd compiles the app host with Rust {HOST_MSRV} \
                 or newer: install Rust from {RUST_INSTALL_URL}"
            );
        }
    }
    let output = run_spec(
        CommandSpec::new("rustc")
            .args(["--version", "--verbose"])
            .cwd(build_dir)
            .allow_failure(true),
    )
    .with_context(|| format!("run `rustc --version`; install Rust from {RUST_INSTALL_URL}"))?;
    if !output.success() {
        bail!(
            "`rustc --version` failed with code {}; install Rust from {RUST_INSTALL_URL}",
            output.exit_code()
        );
    }
    let (version_output, host) = parse_verbose_version(output.stdout_trimmed())?;
    let rustc = check_rustc_version(&version_output)?;
    Ok(RustToolchain {
        rustc,
        version_output,
        host,
    })
}

/// Split `rustc --version --verbose` output into its first line (the plain
/// `rustc --version` line) and the `host:` triple. A compiler that reports no
/// host is an error: the triple is passed to `cargo --target`.
fn parse_verbose_version(output: &str) -> Result<(String, String)> {
    let version = output.lines().next().unwrap_or_default().trim().to_owned();
    let host = output
        .lines()
        .find_map(|line| line.strip_prefix("host:"))
        .map(str::trim)
        .filter(|host| !host.is_empty())
        .context("`rustc -vV` reported no host triple")?
        .to_owned();
    Ok((version, host))
}

/// Parse `rustc --version` output (`rustc 1.95.0 (hash date)`) and require
/// [`HOST_MSRV`]. A nightly or beta of the minimum version is accepted.
fn check_rustc_version(output: &str) -> Result<Version> {
    let found = output
        .split_whitespace()
        .nth(1)
        .and_then(|version| Version::parse(version).ok())
        .with_context(|| format!("unexpected `rustc --version` output: {output:?}"))?;
    if Version::new(found.major, found.minor, found.patch) < minimum_version() {
        bail!(
            "Rust {found} is too old: Tynd compiles the app host with Rust {HOST_MSRV} or newer; \
             run `rustup update` or install Rust from {RUST_INSTALL_URL}"
        );
    }
    Ok(found)
}

fn minimum_version() -> Version {
    let mut parts = HOST_MSRV
        .split('.')
        .map(|part| part.parse::<u64>().unwrap_or(0));
    Version::new(
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn msrv_matches_the_workspace_rust_version() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.toml");
        let manifest: toml::Table = std::fs::read_to_string(root)
            .expect("read root Cargo.toml")
            .parse()
            .expect("parse root Cargo.toml");
        assert_eq!(
            manifest["workspace"]["package"]["rust-version"].as_str(),
            Some(HOST_MSRV)
        );
        assert_eq!(minimum_version(), Version::new(1, 95, 0));
    }

    #[test]
    fn accepts_the_minimum_and_newer_rustc() {
        for output in [
            "rustc 1.95.0 (abcdef012 2026-01-01)",
            "rustc 1.95.0-nightly (abcdef012 2026-01-01)",
            "rustc 1.96.1 (abcdef012 2026-03-01)",
            "rustc 2.0.0",
        ] {
            assert!(check_rustc_version(output).is_ok(), "{output}");
        }
    }

    #[test]
    fn rejects_an_older_rustc_with_the_install_hint() {
        let error = check_rustc_version("rustc 1.94.1 (abcdef012 2025-12-01)")
            .expect_err("too old")
            .to_string();
        assert!(error.contains("1.94.1"), "{error}");
        assert!(error.contains(HOST_MSRV), "{error}");
        assert!(error.contains(RUST_INSTALL_URL), "{error}");
    }

    #[test]
    fn verbose_version_yields_the_version_line_and_the_host_triple() {
        let (version, host) = parse_verbose_version(
            "rustc 1.95.0 (abcdef012 2026-01-01)\nbinary: rustc\nhost: x86_64-pc-windows-msvc\nrelease: 1.95.0",
        )
        .expect("host triple");
        assert_eq!(version, "rustc 1.95.0 (abcdef012 2026-01-01)");
        assert_eq!(host, "x86_64-pc-windows-msvc");
    }

    #[test]
    fn verbose_version_without_a_host_triple_is_an_error() {
        for output in ["rustc 1.95.0", "rustc 1.95.0\nhost:   "] {
            let error = parse_verbose_version(output)
                .expect_err("no host triple")
                .to_string();
            assert!(error.contains("no host triple"), "{error}");
        }
    }

    #[test]
    fn rejects_unreadable_version_output() {
        assert!(check_rustc_version("").is_err());
        assert!(check_rustc_version("cargo").is_err());
    }
}
