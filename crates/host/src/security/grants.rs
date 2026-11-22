//! Runtime scope grants: paths and URL patterns added to the filesystem and
//! HTTP scopes after startup.
//!
//! Only the trusted backend can call `security.allowPath` / `allowUrl` /
//! `revokePath` / `revokeUrl` / `listGrants`: they are not in the renderer /
//! plugin allow-list of `api::deny_privileged_mutation`. The one other writer
//! is a native file dialog opened with `grantAccess: true`, which grants
//! exactly the paths the user picked.
//!
//! Every grant path is kernel-resolved when it is made, so a later symlink
//! swap cannot redirect it. With `security.persistGrants`, grants are saved to
//! `<data dir>/<app>/security/grants.json` and every entry is re-resolved on
//! load: an entry whose path now resolves elsewhere is dropped, never widened.

use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, LazyLock,
    },
};

use arc_swap::ArcSwap;
use parking_lot::Mutex;
use serde_json::{json, Value};
use tynd_security_policy::{canonicalize_fs, Grants, GrantsSnapshot, PathGrant, MAX_PATH_GRANTS};

static GRANTS: LazyLock<ArcSwap<Grants>> =
    LazyLock::new(|| ArcSwap::from_pointee(Grants::default()));
/// Serializes read-modify-write of [`GRANTS`] and the persisted file.
static WRITE_LOCK: Mutex<()> = Mutex::new(());
static PERSIST: AtomicBool = AtomicBool::new(false);

/// Current grant set (lock-free read).
pub(crate) fn current() -> Arc<Grants> {
    GRANTS.load_full()
}

fn mutate<R>(f: impl FnOnce(&mut Grants) -> Result<R, String>) -> Result<R, String> {
    let _guard = WRITE_LOCK.lock();
    let mut next = (*GRANTS.load_full()).clone();
    let result = f(&mut next)?;
    if PERSIST.load(Ordering::Acquire) {
        save(&next.snapshot())?;
    }
    GRANTS.store(Arc::new(next));
    Ok(result)
}

/// Grant `path` (any spelling; it is kernel-resolved here). A missing path is
/// granted as a file to be created, under its resolved existing ancestor.
pub(crate) fn grant_path(path: &str, recursive: bool, write: bool) -> Result<PathGrant, String> {
    let resolved = crate::security::path_scope::resolve_kernel(path)?;
    let grant = PathGrant {
        path: path_string(&resolved)?,
        directory: resolved.is_dir(),
        recursive,
        write,
    };
    mutate(|grants| grants.add_path(grant.clone()))?;
    Ok(grant)
}

fn path_string(path: &Path) -> Result<String, String> {
    path.to_str()
        .map(str::to_string)
        .ok_or_else(|| format!("security: path '{}' is not valid UTF-8", path.display()))
}

fn revoke_path(path: &str) -> Result<bool, String> {
    // The path may be gone by now: try its resolved and its lexical spelling.
    let resolved = crate::security::path_scope::resolve_kernel(path)
        .ok()
        .and_then(|resolved| resolved.to_str().map(str::to_string));
    let lexical = crate::security::path_scope::absolute_lexical(path)?;
    let lexical = path_string(&lexical)?;
    mutate(|grants| {
        let by_resolved = resolved
            .as_deref()
            .is_some_and(|resolved| grants.revoke_path(resolved));
        Ok(by_resolved || grants.revoke_path(&lexical))
    })
}

/// Grant that covers `lexical` (an absolute, lexically clean path) for reads,
/// with the directory a `Dir` must be opened at to reach it: the granted
/// directory, or the parent of a granted file. Most specific grant first.
#[cfg_attr(
    not(any(feature = "api-fs", feature = "api-vault")),
    allow(
        dead_code,
        reason = "only the Dir-confined fs / vault APIs open granted directories"
    )
)]
pub(crate) fn base_for(lexical: &Path) -> Option<PathBuf> {
    let grants = current();
    let candidate = lexical.to_str()?;
    let grant = grants.covering(candidate, tynd_security_policy::FsAccess::Read)?;
    let path = PathBuf::from(&grant.path);
    if grant.directory {
        Some(path)
    } else {
        path.parent().map(Path::to_path_buf)
    }
}

/// Turn persisted grants on or off (from `security.configure`). Turning them
/// on loads the saved file.
pub(crate) fn set_persist(enabled: bool) {
    PERSIST.store(enabled, Ordering::Release);
    // Create the security directory before any renderer runs, whether or not
    // grants persist, so no renderer can pre-create it (or a case / trailing
    // dot variant of it) inside the app data root.
    if let Some(dir) = security_dir() {
        if let Err(error) = std::fs::create_dir_all(&dir) {
            tracing::warn!(
                target: "tynd::host::security",
                "cannot create the security directory {}: {error}",
                dir.display()
            );
        }
    }
    if !enabled {
        return;
    }
    let Some(file) = grants_file() else {
        return;
    };
    if !is_expected_grants_file(&file) {
        tracing::warn!(
            target: "tynd::host::security",
            "ignoring persisted grants {}: its real path is not the expected one",
            file.display()
        );
        return;
    }
    let snapshot = match std::fs::read(&file) {
        Ok(bytes) => match serde_json::from_slice::<GrantsSnapshot>(&bytes) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                tracing::warn!(
                    target: "tynd::host::security",
                    "ignoring unreadable persisted grants {}: {error}",
                    file.display()
                );
                return;
            },
        },
        Err(_) => return,
    };
    let _guard = WRITE_LOCK.lock();
    let mut next = (*GRANTS.load_full()).clone();
    for grant in revalidate(snapshot.paths) {
        if let Err(error) = next.add_path(grant) {
            tracing::warn!(target: "tynd::host::security", "dropping persisted grant: {error}");
        }
    }
    for url in snapshot.urls {
        if let Err(error) = next.add_url(&url) {
            tracing::warn!(target: "tynd::host::security", "dropping persisted grant: {error}");
        }
    }
    GRANTS.store(Arc::new(next));
}

/// Keep only the persisted path grants that still resolve to the spelling
/// that was granted and keep their file / directory kind.
fn revalidate(paths: Vec<PathGrant>) -> Vec<PathGrant> {
    paths
        .into_iter()
        .take(MAX_PATH_GRANTS)
        .filter(|grant| {
            let still_same = Path::new(&grant.path).is_absolute()
                && crate::security::path_scope::resolve_kernel(&grant.path)
                    .ok()
                    .filter(|resolved| {
                        resolved
                            .to_str()
                            .is_some_and(|now| canonicalize_fs(now) == canonicalize_fs(&grant.path))
                    })
                    .is_some_and(|resolved| {
                        !resolved.exists() || resolved.is_dir() == grant.directory
                    });
            if !still_same {
                tracing::warn!(
                    target: "tynd::host::security",
                    path = %grant.path,
                    "dropping persisted grant: the path no longer resolves to what was granted"
                );
            }
            still_same
        })
        .collect()
}

/// `<data dir>/<app>/security`, `<app>` being the app identifier
/// (`crate::app_dirs`), so grants stay with the app and not with whatever the
/// executable is called. The host's own security state: on the built-in deny
/// list (`security::sensitive`), so no renderer or plugin can read or forge
/// the persisted grants even though `<data dir>/<app>` is a capability root.
pub(crate) fn security_dir() -> Option<PathBuf> {
    Some(crate::app_dirs::app_dir(crate::app_dirs::AppDir::Data)?.join("security"))
}

fn grants_file() -> Option<PathBuf> {
    Some(security_dir()?.join("grants.json"))
}

/// The grants file exists and its kernel-resolved path is exactly
/// `<resolved app data dir>/security/grants.json`, byte for byte: no
/// symlinked or junctioned `security` directory or file, and no case or
/// trailing-dot variant a renderer could have created on a case-insensitive
/// filesystem. A missing file is not loaded either.
fn is_expected_grants_file(file: &Path) -> bool {
    let Some(security) = file.parent() else {
        return false;
    };
    let Some(app_data) = security.parent() else {
        return false;
    };
    let (Ok(real_file), Ok(real_app_data)) =
        (dunce::canonicalize(file), dunce::canonicalize(app_data))
    else {
        return false;
    };
    let expected = real_app_data.join("security").join("grants.json");
    real_file.as_os_str() == expected.as_os_str()
        && std::fs::symlink_metadata(&real_file).is_ok_and(|meta| meta.is_file())
}

fn save(snapshot: &GrantsSnapshot) -> Result<(), String> {
    let file = grants_file().ok_or("security: no data directory to persist grants in")?;
    let dir = file.parent().ok_or("security: invalid grants file path")?;
    std::fs::create_dir_all(dir).map_err(|e| format!("security: persist grants: {e}"))?;
    let body = serde_json::to_vec_pretty(snapshot).map_err(|e| e.to_string())?;
    let mut tmp = tempfile::NamedTempFile::new_in(dir)
        .map_err(|e| format!("security: persist grants: {e}"))?;
    std::io::Write::write_all(&mut tmp, &body)
        .map_err(|e| format!("security: persist grants: {e}"))?;
    tmp.persist(&file)
        .map(drop)
        .map_err(|e| format!("security: persist grants: {e}"))
}

fn required_str<'a>(args: &'a Value, method: &str, key: &str) -> Result<&'a str, String> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("security.{method}: '{key}' must be a non-empty string"))
}

fn optional_bool(args: &Value, method: &str, key: &str) -> Result<bool, String> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(false),
        Some(Value::Bool(value)) => Ok(*value),
        Some(_) => Err(format!("security.{method}: '{key}' must be a boolean")),
    }
}

/// `security.*` grant methods. Backend only (see the module docs).
pub(crate) fn dispatch(method: &str, args: &Value) -> Option<Result<Value, String>> {
    let result = match method {
        "allowPath" => (|| {
            let path = required_str(args, method, "path")?;
            let grant = grant_path(
                path,
                optional_bool(args, method, "recursive")?,
                optional_bool(args, method, "write")?,
            )?;
            serde_json::to_value(grant).map_err(|e| e.to_string())
        })(),
        "revokePath" => required_str(args, method, "path")
            .and_then(revoke_path)
            .map(Value::Bool),
        "allowUrl" => required_str(args, method, "pattern")
            .and_then(|pattern| mutate(|grants| grants.add_url(pattern)))
            .map(|()| Value::Null),
        "revokeUrl" => required_str(args, method, "pattern")
            .and_then(|pattern| mutate(|grants| Ok(grants.revoke_url(pattern))))
            .map(Value::Bool),
        "listGrants" => {
            let snapshot = current().snapshot();
            Ok(json!({ "paths": snapshot.paths, "urls": snapshot.urls }))
        },
        _ => return None,
    };
    Some(result)
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use tynd_security_policy::FsAccess;

    use super::*;
    use crate::security::policy::{check_fs_scoped, with_fs_access};

    fn call(method: &str, args: Value) -> Result<Value, String> {
        dispatch(method, &args).expect("grant method")
    }

    #[test]
    fn grant_methods_validate_their_arguments() {
        assert!(call("allowPath", json!({})).is_err());
        assert!(call("allowPath", json!({ "path": "" })).is_err());
        assert!(call("allowPath", json!({ "path": "/x", "write": "yes" })).is_err());
        assert!(call("allowUrl", json!({ "pattern": "**" })).is_err());
        assert!(call("allowUrl", json!({ "pattern": "ftp://example.com/**" })).is_err());
        assert!(call("revokeUrl", json!({})).is_err());
        assert!(dispatch("configure", &json!({})).is_none());
    }

    #[test]
    fn granted_path_is_resolved_listed_and_revoked() {
        let temp = tempfile::tempdir().expect("tempdir");
        let dir = temp.path().join("picked");
        std::fs::create_dir(&dir).expect("mkdir");
        let spelled = dir.join("..").join("picked");
        let grant = call(
            "allowPath",
            json!({ "path": spelled.to_str().expect("utf-8"), "recursive": true }),
        )
        .expect("grant");
        let resolved = dunce::canonicalize(&dir).expect("canonical");
        assert_eq!(grant["path"], json!(resolved.to_str().expect("utf-8")));
        assert_eq!(grant["directory"], json!(true));
        assert_eq!(grant["write"], json!(false));
        let listed = call("listGrants", Value::Null).expect("list");
        assert!(listed["paths"]
            .as_array()
            .expect("paths")
            .iter()
            .any(|entry| entry["path"] == grant["path"]));
        assert_eq!(
            call(
                "revokePath",
                json!({ "path": dir.to_str().expect("utf-8") })
            )
            .expect("revoke"),
            json!(true)
        );
        assert_eq!(
            call(
                "revokePath",
                json!({ "path": dir.to_str().expect("utf-8") })
            )
            .expect("revoke"),
            json!(false)
        );
    }

    #[test]
    fn outside_the_roots_a_read_only_grant_admits_reads_only() {
        let temp = tempfile::tempdir().expect("tempdir");
        let file = temp.path().join("photo.jpg");
        std::fs::write(&file, b"x").expect("write");
        let file = dunce::canonicalize(&file).expect("canonical");
        let other = dunce::canonicalize(temp.path())
            .expect("canonical")
            .join("other.jpg");
        // `in_roots: false`: the path is only reachable through a grant.
        assert!(check_fs_scoped(&file, false).is_err());
        grant_path(file.to_str().expect("utf-8"), false, false).expect("grant");
        assert!(with_fs_access(FsAccess::Read, || check_fs_scoped(&file, false)).is_ok());
        assert!(
            check_fs_scoped(&file, false).is_err(),
            "writes are the default"
        );
        assert!(with_fs_access(FsAccess::Read, || check_fs_scoped(&other, false)).is_err());
        revoke_path(file.to_str().expect("utf-8")).expect("revoke");
        assert!(with_fs_access(FsAccess::Read, || check_fs_scoped(&file, false)).is_err());
    }

    #[test]
    fn only_the_exact_grants_file_is_loaded() {
        let temp = tempfile::tempdir().expect("tempdir");
        let app_data = temp.path().join("app");
        let expected = app_data.join("security").join("grants.json");
        assert!(!is_expected_grants_file(&expected), "missing file");
        std::fs::create_dir_all(app_data.join("security")).expect("mkdir");
        std::fs::create_dir(&expected).expect("mkdir");
        assert!(!is_expected_grants_file(&expected), "a directory");
        std::fs::remove_dir(&expected).expect("rmdir");
        std::fs::write(&expected, b"{}").expect("write");
        assert!(is_expected_grants_file(&expected));
    }

    #[cfg(windows)]
    #[test]
    fn a_case_variant_security_directory_is_not_loaded() {
        let temp = tempfile::tempdir().expect("tempdir");
        let app_data = temp.path().join("app");
        std::fs::create_dir_all(app_data.join("Security")).expect("mkdir");
        std::fs::write(app_data.join("Security").join("grants.json"), b"{}").expect("write");
        assert!(!is_expected_grants_file(
            &app_data.join("security").join("grants.json")
        ));
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_security_directory_is_not_loaded() {
        let temp = tempfile::tempdir().expect("tempdir");
        let app_data = temp.path().join("app");
        let planted = app_data.join("planted");
        std::fs::create_dir_all(&planted).expect("mkdir");
        std::fs::write(planted.join("grants.json"), b"{}").expect("write");
        std::os::unix::fs::symlink(&planted, app_data.join("security")).expect("symlink");
        assert!(!is_expected_grants_file(
            &app_data.join("security").join("grants.json")
        ));
    }

    #[test]
    fn persisted_entries_that_changed_kind_or_spelling_are_dropped() {
        let temp = tempfile::tempdir().expect("tempdir");
        let file = temp.path().join("was-a-dir");
        std::fs::write(&file, b"x").expect("write");
        let canonical = dunce::canonicalize(&file).expect("canonical");
        let canonical = canonical.to_str().expect("utf-8").to_string();
        let kept = PathGrant {
            path: canonical.clone(),
            directory: false,
            recursive: false,
            write: true,
        };
        let changed_kind = PathGrant {
            directory: true,
            ..kept.clone()
        };
        let other_spelling = PathGrant {
            path: format!("{canonical}-link"),
            ..kept.clone()
        };
        let relative = PathGrant {
            path: "relative/path".into(),
            ..kept.clone()
        };
        let survivors = revalidate(vec![kept.clone(), changed_kind, relative]);
        assert_eq!(survivors, vec![kept]);
        // A missing path still resolves to its own spelling and is kept.
        assert_eq!(
            revalidate(vec![other_spelling.clone()]),
            vec![other_spelling]
        );
    }
}
