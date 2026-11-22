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
//! `configure(...)` at startup. The only later widening is a runtime grant
//! (`security::grants`), which the backend or a user-answered file dialog
//! makes for named paths / URL patterns; deny rules still win over it.

mod dispatch;

use std::{
    cell::Cell,
    path::{Path, PathBuf},
    sync::{Arc, LazyLock},
};

use arc_swap::ArcSwap;
use serde_json::Value;
use tynd_security_policy::{FsAccess, SecurityPolicy as Policy};

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
    let persist_grants = input.persist_grants;
    let fs_allow = std::mem::take(&mut input.fs.allow);
    let http_allow = std::mem::take(&mut input.http.allow);
    let next = Policy::from_input(input, fs_allow, http_allow)
        .map_err(|error| format!("security.configure: {error}"))?;
    POLICY.store(Arc::new(next));
    crate::security::grants::set_persist(persist_grants);
    Ok(Value::Null)
}

thread_local! {
    /// Access the filesystem operation running on this thread needs. Read-only
    /// grants only admit [`FsAccess::Read`]; anything that did not declare a
    /// read is treated as a write.
    static FS_ACCESS: Cell<FsAccess> = const { Cell::new(FsAccess::Write) };
}

/// Run `f` with `access` as the declared filesystem access of this thread.
#[cfg_attr(
    not(feature = "api-fs"),
    allow(dead_code, reason = "only the fs API declares read-only calls")
)]
pub(crate) fn with_fs_access<R>(access: FsAccess, f: impl FnOnce() -> R) -> R {
    struct Restore(FsAccess);
    impl Drop for Restore {
        fn drop(&mut self) {
            FS_ACCESS.with(|slot| slot.set(self.0));
        }
    }
    let _restore = Restore(FS_ACCESS.with(|slot| slot.replace(access)));
    f()
}

fn fs_access() -> FsAccess {
    FS_ACCESS.with(Cell::get)
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
    let (resolved, in_roots) = crate::security::path_scope::resolve_for_policy(path)?;
    check_fs_scoped(&resolved, in_roots)?;
    Ok(resolved)
}

/// Policy check of a kernel-resolved path with the runtime grants, for the
/// access this thread declared. `in_roots`: the path lies inside the
/// capability roots (outside them only a grant admits it).
pub(crate) fn check_fs_scoped(path: &Path, in_roots: bool) -> Result<(), String> {
    let path = path
        .to_str()
        .ok_or_else(|| format!("fs: resolved path '{}' is not valid UTF-8", path.display()))?;
    POLICY.load().check_fs_with_grants(
        path,
        &crate::security::grants::current(),
        fs_access(),
        in_roots,
    )
}

/// Whether an explicit `fs.allow` rule names `path` (outside the capability
/// roots, only such a rule or a grant admits a path).
pub(crate) fn fs_allow_rule_matches(path: &Path) -> bool {
    path.to_str()
        .is_some_and(|path| POLICY.load().fs_allow_rule_matches(path))
}

/// Directory the `fs.allow` rule naming `path` is anchored at.
#[cfg_attr(
    not(any(feature = "api-fs", feature = "api-vault")),
    allow(dead_code, reason = "only the Dir-confined `resolve` opens anchors")
)]
pub(crate) fn fs_allow_anchor(path: &Path) -> Option<PathBuf> {
    POLICY
        .load()
        .fs_allow_anchor(path.to_str()?)
        .map(PathBuf::from)
}

/// The built-in deny list and the configured `fs.deny` rules, for a
/// kernel-resolved path that a separate allow-list (the `asset://` scope)
/// admitted.
pub(crate) fn check_fs_not_denied(path: &Path) -> Result<(), String> {
    crate::security::sensitive::check(path)?;
    let path = path
        .to_str()
        .ok_or_else(|| format!("fs: resolved path '{}' is not valid UTF-8", path.display()))?;
    POLICY.load().check_fs_not_denied(path)
}

pub(crate) fn check_http(url: &str) -> Result<(), String> {
    POLICY
        .load()
        .check_http_with_grants(url, &crate::security::grants::current())
}

pub(crate) fn check_http_configured(url: &str) -> Result<(), String> {
    POLICY
        .load()
        .check_http_configured_with_grants(url, &crate::security::grants::current())
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
