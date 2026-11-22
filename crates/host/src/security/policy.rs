//! Capability-based allow/deny lists for FS + HTTP. The trusted backend
//! installs policy at startup before any WebView is created.
//!
//! Patterns are simple globs: `*` matches any run of characters within a
//! segment, `**` crosses separators. Matching is case-insensitive on
//! Windows for FS, case-sensitive everywhere else.
//!
//! Precedence: deny overrides allow. An empty `allow` list means "allow
//! everything" unless `defaultDeny` is set, in which case it means "deny
//! everything".
//!
//! Policy is declarative: the trusted backend installs it once through
//! `configure(...)` at startup and nothing widens it afterwards.

mod dispatch;

use std::{
    path::{Path, PathBuf},
    sync::{Arc, LazyLock},
};

use arc_swap::ArcSwap;
use serde_json::Value;
use tynd_security_policy::SecurityPolicy as Policy;

pub(crate) use tynd_security_policy::PolicyInput;

pub(crate) use self::dispatch::API;

/// Lock-free policy read via `ArcSwap`: readers stay at a single atomic
/// load + Arc bump even under heavy `check_fs` concurrency. The only writer
/// is `configure(...)`.
static POLICY: LazyLock<ArcSwap<Policy>> =
    LazyLock::new(|| ArcSwap::from_pointee(Policy::default()));
pub(crate) fn configure(args: &Value) -> Result<Value, String> {
    let mut input: PolicyInput = serde_json::from_value(args.clone())
        .map_err(|e| format!("security.configure: invalid shape — {e}"))?;
    let fs_allow = std::mem::take(&mut input.fs.allow);
    let http_allow = std::mem::take(&mut input.http.allow);
    let next = Policy::from_input(input, fs_allow, http_allow)
        .map_err(|error| format!("security.configure: {error}"))?;
    POLICY.store(Arc::new(next));
    Ok(Value::Null)
}

pub(crate) fn check_fs(path: &str) -> Result<(), String> {
    authorize_fs_path(path).map(drop)
}

/// Resolve and authorize a renderer/plugin supplied ambient path.
///
/// Existing paths are kernel-canonicalized before policy matching. Missing
/// destinations use their deepest existing canonical ancestor. Capability
/// roots are enforced even while no deployer policy is configured, so the
/// always-on `security.checkFs` plugin bridge cannot authorize more than the
/// pre-opened FS surface.
///
/// Ambient path APIs cannot make the later third-party open race-free. Callers
/// must use the returned path (never the original spelling); an external local
/// actor with filesystem write access may still replace an ancestor between
/// this check and that open. Host FS operations avoid that gap by using
/// cap-std `Dir`-relative calls through `path_scope::resolve`.
pub(crate) fn authorize_fs_path(path: &str) -> Result<PathBuf, String> {
    let resolved = crate::security::path_scope::resolve_for_policy(path)?;
    check_fs_resolved(&resolved)?;
    Ok(resolved)
}

/// Policy-only half used by `path_scope::resolve` after it has already done
/// kernel resolution and both capability-root checks. Keeping this separate
/// prevents `check_fs -> resolve -> check_fs` recursion.
pub(crate) fn check_fs_resolved(path: &Path) -> Result<(), String> {
    let path = path
        .to_str()
        .ok_or_else(|| format!("fs: resolved path '{}' is not valid UTF-8", path.display()))?;
    POLICY.load().check_fs_resolved(path)
}

pub(crate) fn check_http(url: &str) -> Result<(), String> {
    POLICY.load().check_http(url)
}

pub(crate) fn check_http_configured(url: &str) -> Result<(), String> {
    POLICY.load().check_http_configured(url)
}

pub(super) fn check_sensitive_capability(capability: &str) -> Result<Value, String> {
    let p = POLICY.load();
    p.check_sensitive_capability(capability)?;
    Ok(Value::Null)
}

pub(crate) fn check_process(binary: &str) -> Result<(), String> {
    POLICY.load().check_process(binary)
}

/// Shell-mode check that also refuses any caller-supplied env key: the shell
/// expands `%VAR%` / `$VAR` before the first-token allowlist can matter.
#[cfg(feature = "api-process")]
pub(crate) fn check_shell_exec_with_env<'a>(
    cmd_line: &str,
    env_keys: impl IntoIterator<Item = &'a str>,
) -> Result<(), String> {
    POLICY.load().check_shell_exec_with_env(cmd_line, env_keys)
}
