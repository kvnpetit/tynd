use std::{
    fs,
    io::ErrorKind,
    path::{Component, Path, PathBuf},
    sync::LazyLock,
};

use cap_std::{ambient_authority, fs::Dir};

use crate::app_dirs::{app_dir, AppDir};

// Capability roots are always available, including profiles without the
// renderer-facing `api-fs` feature. Native and Bun plugins can call
// `security.checkFs`; that policy probe must not authorize ambient paths that
// the capability-confined FS API itself could never reach.
//
// Default roots: the app's own `<data>/<app>`, `<config>/<app>`,
// `<cache>/<app>` and `<temp>/<app>` directories, plus the working directory
// under `tynd dev` only (a packaged app's cwd is whatever its launcher chose,
// often `/` or `C:\Windows\System32`). Anything else needs an explicit
// `fs.allow` rule (its wildcard-free leading directory is opened on demand)
// or a runtime grant. The built-in deny list (`security::sensitive`) applies
// on top of all of them. `Dir`-relative operations retain cap-std's
// platform-specific confinement. The separate resolved prefix is used only
// for policy decisions made before third-party libraries that require an
// ambient path.

struct Root {
    /// Trusted root spelling used to map a caller path to a relative `Dir`
    /// path without following caller-controlled symlinks.
    lexical_prefix: PathBuf,
    /// Kernel-resolved spelling used to reject paths whose symlink target
    /// escapes every capability root.
    resolved_prefix: PathBuf,
    // Kept alive even in profiles exposing only `security.checkFs`: these
    // successfully pre-opened directories define the capability roots.
    #[cfg(any(feature = "api-fs", feature = "api-vault"))]
    dir: Dir,
    #[cfg(not(any(feature = "api-fs", feature = "api-vault")))]
    _dir: Dir,
}

static ROOTS: LazyLock<Vec<Root>> = LazyLock::new(build_default_roots);

/// Default root directories, before deduplication and opening: the app's own
/// directories (`crate::app_dirs`), plus the working directory under
/// `tynd dev`.
fn default_root_candidates(dev: bool) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if dev {
        // Never a filesystem root, even in dev: that would be the whole disk.
        if let Some(cwd) = std::env::current_dir()
            .ok()
            .filter(|cwd| lexical_clean(cwd).parent().is_some())
        {
            candidates.push(cwd);
        }
    }
    candidates.extend(AppDir::SCOPE_ROOTS.into_iter().filter_map(app_dir));
    candidates
}

fn build_default_roots() -> Vec<Root> {
    let candidates = default_root_candidates(crate::webview::scheme::dev_mode());

    let mut roots = Vec::new();
    for candidate in candidates {
        let lexical_prefix = lexical_clean(&candidate);
        if roots
            .iter()
            .any(|root: &Root| root.lexical_prefix == lexical_prefix)
        {
            continue;
        }
        if !lexical_prefix.exists() && fs::create_dir_all(&lexical_prefix).is_err() {
            continue;
        }
        let Ok(resolved_prefix) = dunce::canonicalize(&lexical_prefix) else {
            continue;
        };
        let Ok(dir) = Dir::open_ambient_dir(&lexical_prefix, ambient_authority()) else {
            continue;
        };
        roots.push(Root {
            lexical_prefix,
            resolved_prefix,
            #[cfg(any(feature = "api-fs", feature = "api-vault"))]
            dir,
            #[cfg(not(any(feature = "api-fs", feature = "api-vault")))]
            _dir: dir,
        });
    }
    roots
}

/// Lexical cleanup used before any caller-controlled filesystem lookup.
/// This removes `.`/`..` without following symlinks.
fn lexical_clean(path: &Path) -> PathBuf {
    let mut output = PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                output.pop();
            },
            Component::CurDir => {},
            other => output.push(other.as_os_str()),
        }
    }
    output
}

/// Convert caller input into an absolute lexical path without touching it.
/// Exposed for `fs.symlink`, whose relative target is interpreted from the
/// link's parent rather than from the process cwd.
pub(crate) fn absolute_lexical(input: &str) -> Result<PathBuf, String> {
    if input.is_empty() {
        return Err("fs: path cannot be empty".to_string());
    }
    let raw = Path::new(input);
    let absolute = if raw.is_absolute() {
        raw.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|error| format!("fs: cwd unavailable — {error}"))?
            .join(raw)
    };
    Ok(lexical_clean(&absolute))
}

#[cfg(not(windows))]
fn strip_prefix_platform(path: &Path, prefix: &Path) -> Option<PathBuf> {
    path.strip_prefix(prefix).ok().map(Path::to_path_buf)
}

#[cfg(windows)]
fn strip_prefix_platform(path: &Path, prefix: &Path) -> Option<PathBuf> {
    let mut path_components = path.components();
    for prefix_component in prefix.components() {
        let path_component = path_components.next()?;
        if !path_component
            .as_os_str()
            .to_string_lossy()
            .eq_ignore_ascii_case(&prefix_component.as_os_str().to_string_lossy())
        {
            return None;
        }
    }
    Some(path_components.as_path().to_path_buf())
}

fn root_for_lexical(path: &Path) -> Option<(&'static Root, PathBuf)> {
    let mut best: Option<(&Root, PathBuf)> = None;
    for root in ROOTS.iter() {
        let Some(relative) = strip_prefix_platform(path, &root.lexical_prefix) else {
            continue;
        };
        if relative
            .components()
            .any(|component| matches!(component, Component::ParentDir))
        {
            continue;
        }
        let prefix_len = root.lexical_prefix.as_os_str().len();
        if best
            .as_ref()
            .is_none_or(|(current, _)| current.lexical_prefix.as_os_str().len() < prefix_len)
        {
            best = Some((root, relative));
        }
    }
    best
}

fn ensure_lexically_within_root(path: &Path) -> Result<(), String> {
    if root_for_lexical(path).is_some() {
        return Ok(());
    }
    Err(format!(
        "fs: path '{}' is outside capability roots ({})",
        path.display(),
        scope_summary(false)
    ))
}

fn ensure_resolved_within_root(path: &Path) -> Result<(), String> {
    if ROOTS
        .iter()
        .any(|root| strip_prefix_platform(path, &root.resolved_prefix).is_some())
    {
        return Ok(());
    }
    Err(format!(
        "fs: resolved path '{}' escapes capability roots ({})",
        path.display(),
        scope_summary(true)
    ))
}

/// Kernel-resolve an existing path. For a destination that does not exist,
/// resolve its deepest existing ancestor and append only the missing lexical
/// suffix. A dangling symlink is an existing directory entry whose target
/// cannot be canonicalized, so it is rejected instead of being mistaken for a
/// creatable destination.
fn resolve_existing_or_ancestor(path: &Path) -> Result<PathBuf, String> {
    let mut cursor = path.to_path_buf();
    let mut missing = Vec::new();

    loop {
        match fs::symlink_metadata(&cursor) {
            Ok(metadata) => {
                let mut resolved = dunce::canonicalize(&cursor).map_err(|error| {
                    if metadata.file_type().is_symlink() {
                        format!(
                            "fs: dangling or inaccessible symlink '{}': {error}",
                            cursor.display()
                        )
                    } else {
                        format!("fs: canonicalize '{}': {error}", cursor.display())
                    }
                })?;
                for component in missing.iter().rev() {
                    resolved.push(component);
                }
                return Ok(lexical_clean(&resolved));
            },
            Err(error) if error.kind() == ErrorKind::NotFound => {
                let name = cursor.file_name().ok_or_else(|| {
                    format!(
                        "fs: no existing ancestor found while resolving '{}'",
                        path.display()
                    )
                })?;
                missing.push(name.to_os_string());
                if !cursor.pop() {
                    return Err(format!(
                        "fs: no existing ancestor found while resolving '{}'",
                        path.display()
                    ));
                }
            },
            Err(error) => {
                return Err(format!("fs: inspect path '{}': {error}", cursor.display()));
            },
        }
    }
}

/// Resolve a path for policy evaluation and ambient plugin consumers.
///
/// The lexical root check happens before the first caller-controlled kernel
/// lookup, preventing UNC/out-of-root probes. The second root check runs on
/// the kernel-resolved path and catches symlink escapes. For a missing output,
/// the returned path is the resolved existing ancestor plus its missing tail.
///
/// This is a point-in-time authorization, not an open file capability. A
/// separate local process (or trusted native code with ambient filesystem
/// authority) can still replace an ancestor after this function returns and
/// before a third-party library opens the path. Callers must use the returned
/// resolved path, and code requiring a race-free guarantee must instead keep
/// I/O relative to the `Dir` returned by [`resolve`], or accept an already-open
/// descriptor. A compromised renderer cannot create an out-of-root symlink
/// through Tynd's FS API because `fs.symlink` validates both ends separately.
pub(crate) fn resolve_for_policy(input: &str) -> Result<(PathBuf, bool), String> {
    resolve_scoped(input).map(|scoped| (scoped.resolved, scoped.in_roots))
}

/// A caller path after both root checks.
struct Scoped {
    #[cfg_attr(
        not(any(feature = "api-fs", feature = "api-vault")),
        allow(dead_code, reason = "read by the Dir-confined `resolve` only")
    )]
    lexical: PathBuf,
    resolved: PathBuf,
    /// Both spellings lie inside the capability roots. `false`: only a
    /// runtime grant (`security.allowPath`, dialog `grantAccess`) admits it.
    in_roots: bool,
}

/// Lexical then resolved root check. A path outside the capability roots is
/// admitted only when a runtime grant covers it, checked lexically before
/// the first kernel lookup (no out-of-root probe) and again on the resolved
/// spelling (no symlink escape out of the grant).
fn resolve_scoped(input: &str) -> Result<Scoped, String> {
    let lexical = absolute_lexical(input)?;
    crate::security::sensitive::check(&lexical)?;
    let lexical_in_roots = root_for_lexical(&lexical).is_some();
    if !lexical_in_roots && !reachable_outside_roots(&lexical) {
        ensure_lexically_within_root(&lexical)?;
    }
    let resolved = resolve_existing_or_ancestor(&lexical)?;
    crate::security::sensitive::check(&resolved)?;
    let in_roots = lexical_in_roots && ensure_resolved_within_root(&resolved).is_ok();
    if !in_roots && !reachable_outside_roots(&resolved) {
        ensure_resolved_within_root(&resolved)?;
        // Inside the roots again, but reached from a granted or allowed
        // spelling: the grant / rule must cover where the path really is.
        return Err(format!(
            "fs: resolved path '{}' is not covered by the grant or fs.allow rule it was reached through",
            resolved.display()
        ));
    }
    Ok(Scoped {
        lexical,
        resolved,
        in_roots,
    })
}

/// Outside the roots, a runtime grant or an explicit `fs.allow` rule admits
/// a path.
fn reachable_outside_roots(path: &Path) -> bool {
    granted(path) || crate::security::policy::fs_allow_rule_matches(path)
}

fn granted(path: &Path) -> bool {
    path.to_str().is_some_and(|path| {
        crate::security::grants::current().allows_path(path, tynd_security_policy::FsAccess::Read)
    })
}

/// Kernel-resolve any absolute or cwd-relative path without the root checks:
/// existing paths are canonicalized, a missing one resolves its deepest
/// existing ancestor. Only for the trusted grant writers.
pub(crate) fn resolve_kernel(input: &str) -> Result<PathBuf, String> {
    resolve_existing_or_ancestor(&absolute_lexical(input)?)
}

/// Map user input to a pre-opened capability `Dir` and relative path.
///
/// Policy evaluates the kernel-resolved destination, while the actual FS
/// syscall remains relative to the root selected from the original lexical
/// spelling. This deliberately fails conservatively when an alias resolves
/// inside a different root: ambient plugin checks may authorize the resolved
/// location, but host FS operations never escape their selected `Dir`.
#[cfg(any(feature = "api-fs", feature = "api-vault"))]
pub(crate) fn resolve(input: &str) -> Result<(&'static Dir, PathBuf), String> {
    let scoped = resolve_scoped(input)?;
    crate::security::policy::check_fs_scoped(&scoped.resolved, scoped.in_roots)?;
    let lexical = scoped.lexical;

    let (dir, mut relative) = if let Some((root, relative)) = root_for_lexical(&lexical) {
        (&root.dir, relative)
    } else {
        outside_root_dir(&lexical)?
    };
    if relative.as_os_str().is_empty() {
        relative = PathBuf::from(".");
    }
    Ok((dir, relative))
}

/// Directories opened for runtime grants and `fs.allow` anchors, opened
/// once per path and kept for the process lifetime (`resolve` hands out
/// `&'static Dir`). Revoking a grant does not close its `Dir`, but nothing
/// reaches it any more: every `resolve` re-checks the live grants and policy
/// first.
#[cfg(any(feature = "api-fs", feature = "api-vault"))]
static OUTSIDE_DIRS: LazyLock<
    parking_lot::Mutex<std::collections::HashMap<PathBuf, &'static Dir>>,
> = LazyLock::new(Default::default);

/// `Dir` for a path outside the roots: the base of the grant covering it,
/// else the anchor of the `fs.allow` rule naming it.
#[cfg(any(feature = "api-fs", feature = "api-vault"))]
fn outside_root_dir(lexical: &Path) -> Result<(&'static Dir, PathBuf), String> {
    let outside = || {
        format!(
            "fs: path '{}' is outside capability roots ({})",
            lexical.display(),
            scope_summary(false)
        )
    };
    let base = crate::security::grants::base_for(lexical)
        .or_else(|| crate::security::policy::fs_allow_anchor(lexical))
        .ok_or_else(outside)?;
    let relative = strip_prefix_platform(lexical, &base).ok_or_else(outside)?;
    let mut dirs = OUTSIDE_DIRS.lock();
    if let Some(dir) = dirs.get(&base) {
        return Ok((dir, relative));
    }
    if dirs.len() >= 2 * tynd_security_policy::MAX_PATH_GRANTS {
        return Err("fs: too many directories outside the capability roots are open".into());
    }
    let dir = Dir::open_ambient_dir(&base, ambient_authority())
        .map_err(|error| format!("fs: open directory '{}': {error}", base.display()))?;
    let dir: &'static Dir = Box::leak(Box::new(dir));
    dirs.insert(base, dir);
    Ok((dir, relative))
}

/// A fresh directory inside the default `<temp>/<app>` capability root, for
/// tests exercising the root-confined APIs.
#[cfg(all(
    test,
    any(feature = "api-fs", feature = "api-vault", feature = "api-http")
))]
pub(crate) fn scoped_tempdir() -> tempfile::TempDir {
    let base = crate::app_dirs::ensure_app_dir(AppDir::Temp).expect("app temp root");
    tempfile::tempdir_in(base).expect("tempdir")
}

fn scope_summary(resolved: bool) -> String {
    ROOTS
        .iter()
        .map(|root| {
            if resolved {
                root.resolved_prefix.display().to_string()
            } else {
                root.lexical_prefix.display().to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(path: &Path) -> &str {
        path.to_str().expect("utf-8 path")
    }

    #[test]
    fn cwd_is_a_root_only_in_dev() {
        let cwd = std::env::current_dir().expect("cwd");
        assert!(!default_root_candidates(false).contains(&cwd));
        assert!(default_root_candidates(true).contains(&cwd));
    }

    #[test]
    fn default_roots_are_the_app_scoped_directories() {
        let candidates = default_root_candidates(false);
        assert!(
            candidates.len() >= 2,
            "data/config/cache/temp app directories"
        );
        for candidate in &candidates {
            assert_eq!(
                candidate.file_name().and_then(|name| name.to_str()),
                Some(crate::app_dirs::name()),
                "{}",
                candidate.display()
            );
        }
        let home = dirs::home_dir().expect("home dir");
        assert!(!candidates.contains(&home));
        assert!(!candidates.contains(&std::env::temp_dir()));
    }

    #[test]
    fn home_and_other_apps_data_are_outside_the_default_scope() {
        let home = dirs::home_dir().expect("home dir");
        let error = resolve_for_policy(text(&home.join("tynd-not-in-scope.txt")))
            .expect_err("home is not a root");
        assert!(error.contains("outside capability roots"), "{error}");
        let data = dirs::data_dir().expect("data dir");
        assert!(resolve_for_policy(text(&data.join("SomeOtherApp").join("db.sqlite"))).is_err());
        let own = data.join(crate::app_dirs::name()).join("settings.json");
        assert!(resolve_for_policy(text(&own)).is_ok());
    }

    #[test]
    fn a_filesystem_root_is_never_reachable_by_default() {
        let root = std::env::temp_dir()
            .ancestors()
            .last()
            .expect("fs root")
            .join("tynd-root-probe.txt");
        assert!(resolve_for_policy(text(&root)).is_err());
    }

    #[test]
    fn credential_stores_and_the_grants_file_are_denied_inside_the_scope() {
        let home = dirs::home_dir().expect("home dir");
        let error =
            resolve_for_policy(text(&home.join(".ssh").join("id_ed25519"))).expect_err("protected");
        assert!(error.contains("protected location"), "{error}");
        let grants = crate::security::grants::security_dir()
            .expect("data dir")
            .join("grants.json");
        let error = resolve_for_policy(text(&grants)).expect_err("protected");
        assert!(error.contains("protected location"), "{error}");
    }

    #[test]
    fn a_grant_never_reaches_a_protected_file() {
        let temp = tempfile::tempdir().expect("tempdir");
        fs::write(temp.path().join(".netrc"), b"machine x").expect("write");
        fs::write(temp.path().join("notes.txt"), b"x").expect("write");
        let base = dunce::canonicalize(temp.path()).expect("canonical");
        crate::security::grants::grant_path(text(&base), true, false).expect("grant");
        let allowed = tynd_security_policy::FsAccess::Read;
        crate::security::policy::with_fs_access(allowed, || {
            assert!(
                crate::security::policy::authorize_fs_path(text(&base.join("notes.txt"))).is_ok()
            );
            let error = crate::security::policy::authorize_fs_path(text(&base.join(".netrc")))
                .expect_err("protected");
            assert!(error.contains("protected location"), "{error}");
        });
        let revoke = serde_json::json!({ "path": text(&base) });
        crate::security::grants::dispatch("revokePath", &revoke)
            .expect("grant method")
            .expect("revoke");
    }
}
