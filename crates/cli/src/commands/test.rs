//! `tynd test`: run the project's end-to-end tests with `bun test`.
//!
//! Tests drive the app through `@tynd/test`, which launches `tynd dev` (the
//! debug socket is on by default) and talks to it over the debug protocol.
//! This command only prepares the environment those tests rely on.

use std::{
    ffi::OsString,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};

use crate::{
    args::TestArgs,
    config,
    process::{self, CommandSpec},
};

use crate::fsx::absolute;
use crate::output::{display_path, step};

/// Environment variable naming the `tynd` executable `@tynd/test` launches.
pub(super) const ENV_CLI: &str = "TYND_TEST_CLI";
/// Environment variable naming the project root under test.
pub(super) const ENV_PROJECT: &str = "TYND_TEST_PROJECT";
/// Environment variable carrying the per-test timeout, so `@tynd/test` keeps its launch timeout below it.
pub(super) const ENV_TEST_TIMEOUT: &str = "TYND_TEST_TIMEOUT_MS";
/// Environment variable that makes `@tynd/test` rewrite screenshot baselines.
pub(super) const ENV_UPDATE_SNAPSHOTS: &str = "TYND_UPDATE_SNAPSHOTS";

pub(super) struct TestRequest {
    pub(super) root: PathBuf,
    pub(super) timeout_ms: u64,
    pub(super) update_snapshots: bool,
    pub(super) args: Vec<String>,
    pub(super) cli: Option<PathBuf>,
}

pub(super) fn request(args: TestArgs) -> Result<TestRequest> {
    let root = config::find_project_root(&absolute(&args.cwd)?)?;
    Ok(TestRequest {
        root,
        timeout_ms: args.timeout_ms,
        update_snapshots: args.update_snapshots,
        args: args.args,
        cli: std::env::current_exe().ok(),
    })
}

pub(super) fn test(request: &TestRequest) -> Result<()> {
    let bun = process::find_executable("bun").context("Bun executable not found")?;
    step(format!("bun test ({})", display_path(&request.root)));
    process::run_streaming(runner_spec(request, &bun)).map(|_| ())
}

fn forwards_timeout(args: &[String]) -> bool {
    args.iter()
        .any(|argument| argument == "--timeout" || argument.starts_with("--timeout="))
}

/// Value of a user-supplied `--timeout N` / `--timeout=N`, when it parses.
fn forwarded_timeout(args: &[String]) -> Option<u64> {
    args.iter().enumerate().find_map(|(index, argument)| {
        if let Some(value) = argument.strip_prefix("--timeout=") {
            value.parse().ok()
        } else if argument == "--timeout" {
            args.get(index + 1)?.parse().ok()
        } else {
            None
        }
    })
}

fn runner_spec(request: &TestRequest, bun: &Path) -> CommandSpec {
    let mut spec = CommandSpec::new(bun.as_os_str())
        .arg("test")
        .cwd(&request.root)
        .without_timeout()
        .env("TYND_TEST", "1")
        .env(ENV_PROJECT, request.root.as_os_str());
    let mut effective_timeout = Some(request.timeout_ms);
    if forwards_timeout(&request.args) {
        effective_timeout = forwarded_timeout(&request.args);
    } else {
        spec = spec.args([
            OsString::from("--timeout"),
            OsString::from(request.timeout_ms.to_string()),
        ]);
    }
    if let Some(timeout) = effective_timeout {
        spec = spec.env(ENV_TEST_TIMEOUT, timeout.to_string());
    }
    spec = spec.args(request.args.iter().map(OsString::from));
    if let Some(cli) = &request.cli {
        spec = spec.env(ENV_CLI, cli.as_os_str());
    }
    if request.update_snapshots {
        spec = spec.env(ENV_UPDATE_SNAPSHOTS, "1");
    }
    if std::env::var_os("TYND_ENV").is_none() {
        spec = spec.env("TYND_ENV", "test");
    }
    spec
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(args: &[&str], update: bool) -> TestRequest {
        TestRequest {
            root: PathBuf::from("project"),
            timeout_ms: 1234,
            update_snapshots: update,
            args: args.iter().map(ToString::to_string).collect(),
            cli: Some(PathBuf::from("tynd-bin")),
        }
    }

    fn env_value(spec: &CommandSpec, key: &str) -> Option<String> {
        spec.env
            .get(&OsString::from(key))
            .and_then(Option::as_ref)
            .map(|value| value.to_string_lossy().into_owned())
    }

    #[test]
    fn builds_bun_test_with_default_timeout_and_forwarded_args() {
        let spec = runner_spec(&request(&["a.test.ts", "--bail"], false), Path::new("bun"));
        let args: Vec<_> = spec
            .args
            .iter()
            .map(|value| value.to_string_lossy().into_owned())
            .collect();
        assert_eq!(args, ["test", "--timeout", "1234", "a.test.ts", "--bail"]);
        assert_eq!(spec.cwd.as_deref(), Some(Path::new("project")));
        assert_eq!(env_value(&spec, "TYND_TEST").as_deref(), Some("1"));
        assert_eq!(env_value(&spec, ENV_CLI).as_deref(), Some("tynd-bin"));
        assert_eq!(env_value(&spec, ENV_PROJECT).as_deref(), Some("project"));
        assert_eq!(env_value(&spec, ENV_UPDATE_SNAPSHOTS), None);
    }

    #[test]
    fn keeps_a_user_supplied_timeout() {
        for flag in [["--timeout", "9"], ["--timeout=9", "x"]] {
            let spec = runner_spec(&request(&flag, false), Path::new("bun"));
            let timeouts = spec
                .args
                .iter()
                .filter(|value| value.to_string_lossy().starts_with("--timeout"))
                .count();
            assert_eq!(timeouts, 1);
        }
    }

    #[test]
    fn exports_the_effective_timeout_for_the_launch_helper() {
        let spec = runner_spec(&request(&["a.test.ts"], false), Path::new("bun"));
        assert_eq!(env_value(&spec, ENV_TEST_TIMEOUT).as_deref(), Some("1234"));
        let spec = runner_spec(&request(&["--timeout", "9000"], false), Path::new("bun"));
        assert_eq!(env_value(&spec, ENV_TEST_TIMEOUT).as_deref(), Some("9000"));
        let spec = runner_spec(&request(&["--timeout=7000"], false), Path::new("bun"));
        assert_eq!(env_value(&spec, ENV_TEST_TIMEOUT).as_deref(), Some("7000"));
        let spec = runner_spec(&request(&["--timeout", "soon"], false), Path::new("bun"));
        assert_eq!(env_value(&spec, ENV_TEST_TIMEOUT), None);
    }

    #[test]
    fn update_snapshots_sets_the_baseline_variable() {
        let spec = runner_spec(&request(&[], true), Path::new("bun"));
        assert_eq!(env_value(&spec, ENV_UPDATE_SNAPSHOTS).as_deref(), Some("1"));
    }
}
