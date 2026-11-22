use std::{
    fs,
    io::ErrorKind,
    path::{Component, Path, PathBuf},
    sync::LazyLock,
};

use cap_std::{ambient_authority, fs::Dir};

// Capability roots are always available, including profiles without the
// renderer-facing `api-fs` feature. Native and Bun plugins can call
// `security.checkFs`; that policy probe must not authorize ambient paths that
// the capability-confined FS API itself could never reach.
//
// Defaults: cwd, data_dir, config_dir, temp_dir, home_dir. `Dir`-relative
// operations retain cap-std's platform-specific confinement. The separate
// resolved prefix is used only for policy decisions made before third-party
// libraries that require an ambient path.

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

fn build_default_roots() -> Vec<Root> {
    let mut candidates = Vec::new();
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd);
    }
    if let Some(path) = dirs::data_dir() {
        candidates.push(path);
    }
    if let Some(path) = dirs::config_dir() {
        candidates.push(path);
    }
    candidates.push(std::env::temp_dir());
    if let Some(path) = dirs::home_dir() {
        candidates.push(path);
    }

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
pub(crate) fn resolve_for_policy(input: &str) -> Result<PathBuf, String> {
    let lexical = absolute_lexical(input)?;
    ensure_lexically_within_root(&lexical)?;
    let resolved = resolve_existing_or_ancestor(&lexical)?;
    ensure_resolved_within_root(&resolved)?;
    Ok(resolved)
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
    let lexical = absolute_lexical(input)?;
    ensure_lexically_within_root(&lexical)?;
    let resolved = resolve_existing_or_ancestor(&lexical)?;
    ensure_resolved_within_root(&resolved)?;
    crate::security::policy::check_fs_resolved(&resolved)?;

    let Some((root, mut relative)) = root_for_lexical(&lexical) else {
        return Err(format!(
            "fs: path '{}' is outside capability roots ({})",
            lexical.display(),
            scope_summary(false)
        ));
    };
    if relative.as_os_str().is_empty() {
        relative = PathBuf::from(".");
    }
    Ok((&root.dir, relative))
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
