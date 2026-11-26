use std::{
    collections::HashMap,
    fmt::Write as _,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    sync::LazyLock,
};

use parking_lot::Mutex;
use serde_json::{json, Value};

use super::{blake3_file, hashes_match};

#[cfg(target_os = "macos")]
const MAX_APP_ARCHIVE_ENTRIES: usize = 200_000;
#[cfg(target_os = "macos")]
const MAX_APP_ARCHIVE_UNCOMPRESSED_BYTES: u64 = 4 * 1024 * 1024 * 1024;

/// Concurrency gate for `install()`. Two parallel calls would race on
/// `self_replace` and the relaunch spawn, briefly leaving two updated
/// binaries alive and competing for the single-instance lock. The
/// `Mutex` rejects overlap; `INSTALL_DONE` rejects retries after a
/// successful swap (which is followed by `process::exit`, so any
/// post-swap state is technically unreachable but the latch documents
/// the invariant).
static INSTALL_LOCK: Mutex<()> = Mutex::new(());
static INSTALL_DONE: AtomicBool = AtomicBool::new(false);
static VERIFIED_ARTIFACTS: LazyLock<Mutex<HashMap<String, VerifiedArtifact>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
const MAX_STAGED_ARTIFACTS: usize = 64;

struct VerifiedArtifact {
    /// Webview identity that staged the artifact (`None` for the backend); only
    /// it may install it.
    owner: Option<crate::ipc::owner::Owner>,
    path: PathBuf,
    hash: String,
    format: String,
}

impl Drop for VerifiedArtifact {
    fn drop(&mut self) {
        match std::fs::remove_file(&self.path) {
            Ok(()) => {},
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
            Err(error) => {
                let staged = self
                    .path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("<staged>");
                tracing::debug!(
                    target: "tynd::updater",
                    staged,
                    "verified artifact cleanup failed: {error}"
                );
            },
        }
    }
}

/// Register a payload only after its Ed25519 verification succeeds. The
/// returned capability is random, process-local and single-use; renderer code
/// can no longer nominate an arbitrary path/hash pair for self replacement.
pub(super) fn register_verified_artifact(
    path: &Path,
    hash: String,
    format: &str,
) -> Result<String, String> {
    let mut registry = VERIFIED_ARTIFACTS.lock();
    registry.retain(|_, artifact| artifact.path.exists());
    if registry.len() >= MAX_STAGED_ARTIFACTS {
        return Err(format!(
            "updater: too many staged artifacts ({MAX_STAGED_ARTIFACTS}); install or restart first"
        ));
    }

    for _ in 0..8 {
        let mut bytes = [0_u8; 32];
        getrandom::fill(&mut bytes)
            .map_err(|e| format!("updater: secure handle generation failed: {e}"))?;
        let mut handle = String::with_capacity(bytes.len() * 2);
        for byte in bytes {
            write!(&mut handle, "{byte:02x}")
                .expect("formatting into an in-memory String cannot fail");
        }
        if !registry.contains_key(&handle) {
            // Registered under the lock `release_owner` takes: a stage call that
            // was still running when its webview committed a new document is
            // backend-owned from the start, as if the release had seen it.
            let owner = crate::ipc::owner::current().filter(crate::ipc::owner::Owner::is_current);
            registry.insert(
                handle.clone(),
                VerifiedArtifact {
                    owner,
                    path: path.to_path_buf(),
                    hash,
                    format: format.to_string(),
                },
            );
            return Ok(handle);
        }
    }
    Err("updater: failed to allocate a unique verified-artifact handle".into())
}

/// The document of webview `label` is gone: re-own the artifacts it staged to
/// the backend, so a later document of the label cannot install them (unless it
/// is app-local, like any backend resource).
pub(super) fn release_owner(label: &str) {
    for artifact in VERIFIED_ARTIFACTS.lock().values_mut() {
        if artifact
            .owner
            .as_ref()
            .is_some_and(|owner| owner.label() == label)
        {
            artifact.owner = None;
        }
    }
}

/// Consume the artifact behind `handle` if the caller may use it. Another
/// webview's handle answers like an unknown one and stays registered.
fn take_verified_artifact(handle: &str) -> Option<VerifiedArtifact> {
    let mut registry = VERIFIED_ARTIFACTS.lock();
    if registry
        .get(handle)
        .is_some_and(|artifact| crate::ipc::owner::permits(artifact.owner.as_ref()))
    {
        registry.remove(handle)
    } else {
        None
    }
}

pub(super) fn install(args: &Value) -> Result<Value, String> {
    if INSTALL_DONE.load(Ordering::Acquire) {
        return Err("updater.install: an install already completed in this process".to_string());
    }

    let _guard = INSTALL_LOCK
        .try_lock()
        .ok_or("updater.install: another install is already in progress")?;
    let handle = args
        .get("handle")
        .and_then(Value::as_str)
        .ok_or("updater.install: missing verified artifact 'handle'")?;
    let relaunch = args
        .get("relaunch")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    // Consume before validation so capabilities are single-use even when a
    // staged file was tampered with. `VerifiedArtifact::drop` removes the
    // staged payload on every return path, including validation/relaunch
    // failures and successful swaps (where the path may already have moved).
    let artifact = take_verified_artifact(handle)
        .ok_or("updater.install: unknown, expired, or already-used verified artifact handle")?;
    let new_path_buf = &artifact.path;
    let expected_hash = &artifact.hash;
    let artifact_format = &artifact.format;

    let current =
        std::env::current_exe().map_err(|e| format!("updater.install: current_exe: {e}"))?;

    if !new_path_buf.exists() {
        return Err("updater.install: verified staged file no longer exists".to_string());
    }

    verify_staged_hash(new_path_buf, expected_hash)?;

    let new_path = new_path_buf
        .to_str()
        .ok_or("updater.install: verified staged path is not UTF-8")?;

    #[cfg(target_os = "windows")]
    {
        require_binary_format(artifact_format)?;
        install_windows(new_path, &current, relaunch)
    }

    #[cfg(target_os = "linux")]
    {
        require_binary_format(artifact_format)?;
        install_linux(new_path, &current, relaunch)
    }

    #[cfg(target_os = "macos")]
    {
        install_macos(new_path, &current, relaunch, artifact_format)
    }

    #[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
    {
        Err("updater.install: unsupported target OS".to_string())
    }
}

/// Re-hash the staged file and refuse the swap unless it still matches the
/// hash recorded when it was verified.
fn verify_staged_hash(staged_path: &Path, expected_hash: &str) -> Result<(), String> {
    // Re-verify the on-disk file matches the verified-bytes hash from
    // `download_and_verify`. Length-checked + constant-time compare
    // because mismatched-length hex would otherwise short-circuit the
    // memory equality.
    let actual_hash = blake3_file(staged_path)
        .map_err(|e| format!("updater.install: hash recompute failed: {e}"))?;
    if !hashes_match(expected_hash, &actual_hash) {
        // Hard failure — surface it loudly so a deployer sees the
        // tampering attempt in logs, then refuse the swap. Log only the
        // basename of the staged file: the full path lives under
        // `%LOCALAPPDATA%\Tynd\…` on Windows and leaks the OS user name.
        let staged = staged_path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("<staged>");
        tracing::error!(
            target: "tynd::updater",
            expected = %expected_hash,
            actual = %actual_hash,
            staged = %staged,
            "tampering detected: staged update file does not match verified hash"
        );
        return Err(format!(
            "updater.install: tampering detected — staged file hash {actual_hash} does not match \
             verified hash {expected_hash}; refusing self_replace"
        ));
    }
    Ok(())
}

#[cfg(any(target_os = "windows", target_os = "linux"))]
fn require_binary_format(format: &str) -> Result<(), String> {
    if format == "binary" {
        Ok(())
    } else {
        Err(format!(
            "updater.install: artifact format '{format}' is not supported on this platform"
        ))
    }
}

/// Refuse a download that `install` could never apply. The swap writes next
/// to the running executable (Windows / Linux) or next to the `.app` bundle
/// (macOS); an app installed per machine or by a system package manager has
/// that directory read-only for the user, so the download and verification
/// would succeed and the swap fail with a raw permission error.
pub(super) fn ensure_self_updatable(operation: &str) -> Result<(), String> {
    let current =
        std::env::current_exe().map_err(|e| format!("updater.{operation}: current_exe: {e}"))?;
    let dir = install_dir(&current).ok_or_else(|| {
        format!("updater.{operation}: current executable has no parent directory")
    })?;
    tempfile::Builder::new()
        .prefix(".tynd-update-probe-")
        .tempfile_in(&dir)
        .map(drop)
        .map_err(|e| {
            format!(
                "updater.{operation}: not a self-updatable install: '{}' is not writable by the \
                 current user ({e}); update through the installer or package manager that \
                 installed the app",
                dir.display()
            )
        })
}

/// Directory the swap writes into: the `.app` bundle's parent on macOS, the
/// executable's own directory elsewhere.
fn install_dir(current: &Path) -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    if let Some(app) = find_app_ancestor(current) {
        return app.parent().map(Path::to_path_buf);
    }
    current.parent().map(Path::to_path_buf)
}

#[cfg(target_os = "macos")]
pub(super) fn find_app_ancestor(path: &Path) -> Option<PathBuf> {
    path.ancestors()
        .find(|candidate| candidate.extension().is_some_and(|ext| ext == "app"))
        .map(Path::to_path_buf)
}

/// Extract exactly one `.app` tree from an updater ZIP.
///
/// `enclosed_name` rejects absolute paths and `..` traversal. Symlinks (needed
/// by `.framework` bundles: `Versions/Current`, `Headers`, ...) are collected
/// while the regular files are written and created only afterwards, so no later
/// entry can be written through a link. Each link must have a relative target
/// and its parent directory must resolve inside the extraction root before it
/// is created; once every link exists, each must resolve (symlinks followed)
/// to a path still inside that root, otherwise the whole archive is refused.
#[cfg(target_os = "macos")]
pub(super) fn extract_app_archive(
    archive_path: &Path,
    destination: &Path,
) -> Result<PathBuf, String> {
    use std::{
        collections::BTreeSet,
        fs::{self, OpenOptions},
        io,
    };

    let file = fs::File::open(archive_path)
        .map_err(|e| format!("updater.install: open app archive: {e}"))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| format!("updater.install: read app archive: {e}"))?;
    if archive.len() > MAX_APP_ARCHIVE_ENTRIES {
        return Err(format!(
            "updater.install: app archive has {} entries (max {MAX_APP_ARCHIVE_ENTRIES})",
            archive.len()
        ));
    }

    let mut app_roots = BTreeSet::new();
    let mut uncompressed = 0_u64;
    for index in 0..archive.len() {
        let entry = archive
            .by_index(index)
            .map_err(|e| format!("updater.install: inspect archive entry {index}: {e}"))?;
        let name = entry.enclosed_name().ok_or_else(|| {
            format!(
                "updater.install: unsafe archive path '{}'",
                zip_entry_label(entry.name())
            )
        })?;
        uncompressed = uncompressed
            .checked_add(entry.size())
            .ok_or("updater.install: app archive size overflow")?;
        if uncompressed > MAX_APP_ARCHIVE_UNCOMPRESSED_BYTES {
            return Err(format!(
                "updater.install: expanded app archive exceeds {MAX_APP_ARCHIVE_UNCOMPRESSED_BYTES}-byte cap"
            ));
        }

        let mut root = PathBuf::new();
        for component in name.components() {
            root.push(component.as_os_str());
            if component
                .as_os_str()
                .to_string_lossy()
                .to_ascii_lowercase()
                .ends_with(".app")
            {
                app_roots.insert(root.clone());
                break;
            }
        }
    }
    if app_roots.len() != 1 {
        return Err(format!(
            "updater.install: app archive must contain exactly one .app bundle (found {})",
            app_roots.len()
        ));
    }
    let app_root = app_roots
        .into_iter()
        .next()
        .ok_or("updater.install: app archive contains no .app bundle")?;
    let app_parent = app_root.parent().unwrap_or_else(|| Path::new(""));

    let mut symlinks: Vec<(PathBuf, String)> = Vec::new();
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|e| format!("updater.install: read archive entry {index}: {e}"))?;
        let name = entry.enclosed_name().ok_or_else(|| {
            format!(
                "updater.install: unsafe archive path '{}'",
                zip_entry_label(entry.name())
            )
        })?;
        if !name.starts_with(&app_root) {
            continue;
        }
        let relative = name
            .strip_prefix(app_parent)
            .map_err(|e| format!("updater.install: strip app archive prefix: {e}"))?;
        let output = destination.join(relative);
        if entry.is_dir() {
            fs::create_dir_all(&output)
                .map_err(|e| format!("updater.install: create app directory: {e}"))?;
            continue;
        }
        let parent = output
            .parent()
            .ok_or("updater.install: archive file has no parent")?;
        fs::create_dir_all(parent)
            .map_err(|e| format!("updater.install: create app parent: {e}"))?;
        if entry
            .unix_mode()
            .is_some_and(|mode| mode & 0o170_000 == 0o120_000)
        {
            let mut target = String::new();
            io::Read::read_to_string(
                &mut io::Read::take(&mut entry, MAX_SYMLINK_TARGET_BYTES + 1),
                &mut target,
            )
            .map_err(|e| format!("updater.install: read symlink target: {e}"))?;
            if target.len() as u64 > MAX_SYMLINK_TARGET_BYTES
                || !symlink_target_is_relative(&target)
            {
                return Err(format!(
                    "updater.install: unsafe symlink target in '{}'",
                    zip_entry_label(entry.name())
                ));
            }
            symlinks.push((output, target));
            continue;
        }
        let mut output_file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&output)
            .map_err(|e| format!("updater.install: create extracted app file: {e}"))?;
        io::copy(&mut entry, &mut output_file)
            .map_err(|e| format!("updater.install: extract app file: {e}"))?;

        #[cfg(unix)]
        if let Some(mode) = entry.unix_mode() {
            use std::os::unix::fs::PermissionsExt as _;
            fs::set_permissions(&output, fs::Permissions::from_mode(mode & 0o777))
                .map_err(|e| format!("updater.install: restore app file mode: {e}"))?;
        }
    }

    let extracted = destination.join(
        app_root
            .file_name()
            .ok_or("updater.install: malformed .app root")?,
    );
    create_confined_symlinks(&symlinks, &extracted)?;
    if !extracted.join("Contents").join("Info.plist").is_file() {
        return Err("updater.install: extracted bundle has no Contents/Info.plist".into());
    }
    Ok(extracted)
}

/// Longest symlink target accepted from an archive.
#[cfg(target_os = "macos")]
const MAX_SYMLINK_TARGET_BYTES: u64 = 1024;

/// A symlink target is acceptable only when it is a non-empty relative path
/// without NUL. `..` is allowed (frameworks use `../Versions/A/...`), but where
/// the link finally lands is verified physically after creation.
#[cfg(any(target_os = "macos", test))]
fn symlink_target_is_relative(target: &str) -> bool {
    use std::path::Component;
    !target.is_empty()
        && !target.contains('\0')
        && Path::new(target).components().all(|c| {
            matches!(
                c,
                Component::Normal(_) | Component::CurDir | Component::ParentDir
            )
        })
}

/// Create the archive's symlinks (after every regular file is on disk) and
/// prove each one stays inside `root`. A link whose parent directory resolves
/// outside `root` (reached through an earlier link) is never created.
#[cfg(target_os = "macos")]
fn create_confined_symlinks(links: &[(PathBuf, String)], root: &Path) -> Result<(), String> {
    use std::{fs, os::unix::fs::symlink};

    if links.is_empty() {
        return Ok(());
    }
    let root = fs::canonicalize(root)
        .map_err(|e| format!("updater.install: resolve extraction root: {e}"))?;
    for (link, target) in links {
        let parent = link
            .parent()
            .ok_or("updater.install: symlink has no parent directory")?;
        let parent = fs::canonicalize(parent)
            .map_err(|e| format!("updater.install: resolve symlink parent: {e}"))?;
        if !parent.starts_with(&root) {
            return Err("updater.install: symlink parent escapes the app bundle".into());
        }
        symlink(target, link).map_err(|e| format!("updater.install: create symlink: {e}"))?;
    }
    for (link, _) in links {
        let resolved = fs::canonicalize(link)
            .map_err(|e| format!("updater.install: symlink does not resolve: {e}"))?;
        if !resolved.starts_with(&root) {
            return Err(format!(
                "updater.install: symlink '{}' escapes the app bundle",
                link.display()
            ));
        }
    }
    Ok(())
}

/// Printable entry name for error messages. zip 9 returns names fallibly
/// (`ZipFile::name` can fail on undecodable names).
#[cfg(target_os = "macos")]
fn zip_entry_label(name: zip::result::ZipResult<std::borrow::Cow<'_, str>>) -> String {
    name.map_or_else(
        |e| format!("<unreadable name: {e}>"),
        std::borrow::Cow::into_owned,
    )
}

#[cfg(target_os = "macos")]
fn install_macos(
    archive_path: &str,
    current_exe: &Path,
    relaunch: bool,
    format: &str,
) -> Result<Value, String> {
    use std::process::Command;

    if format != "app-archive" {
        return Err(
            "updater.install: macOS requires an authenticated 'app-archive' payload".into(),
        );
    }
    let current_app = find_app_ancestor(current_exe)
        .ok_or("updater.install: current executable is not running inside a macOS .app bundle")?;
    let app_parent = current_app
        .parent()
        .ok_or("updater.install: installed .app has no parent directory")?;
    let extraction = tempfile::Builder::new()
        .prefix(".tynd-update-")
        .tempdir_in(app_parent)
        .map_err(|e| format!("updater.install: create same-volume staging directory: {e}"))?;
    let staged_app = extract_app_archive(Path::new(archive_path), extraction.path())?;

    let current_id = macos_bundle_id(&current_app)?;
    let staged_id = macos_bundle_id(&staged_app)?;
    if current_id != staged_id {
        return Err(format!(
            "updater.install: bundle identifier mismatch (installed '{current_id}', update '{staged_id}')"
        ));
    }
    let verified = Command::new("/usr/bin/codesign")
        .args(["--verify", "--deep", "--strict"])
        .arg(&staged_app)
        .status()
        .map_err(|e| format!("updater.install: launch codesign verification: {e}"))?;
    if !verified.success() {
        return Err(format!(
            "updater.install: staged .app failed codesign verification ({verified})"
        ));
    }

    let backup = app_parent.join(format!(
        ".{}.tynd-backup-{}-{}",
        current_app
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("app"),
        std::process::id(),
        super::simple_rand()
    ));
    std::fs::rename(&current_app, &backup)
        .map_err(|e| format!("updater.install: move installed .app to rollback path: {e}"))?;
    if let Err(error) = std::fs::rename(&staged_app, &current_app) {
        if let Err(rollback) = std::fs::rename(&backup, &current_app) {
            return Err(format!(
                "updater.install: app swap failed ({error}); rollback also failed ({rollback}); backup remains at {}",
                backup.display()
            ));
        }
        return Err(format!(
            "updater.install: app swap failed ({error}); previous bundle restored"
        ));
    }

    if let Err(error) = spawn_macos_cleanup(current_exe, &backup, relaunch) {
        let failed_update = extraction.path().join("failed-update.app");
        let rollback_result = std::fs::rename(&current_app, &failed_update)
            .and_then(|()| std::fs::rename(&backup, &current_app));
        return match rollback_result {
            Ok(()) => Err(format!(
                "updater.install: relaunch helper failed ({error}); previous bundle restored"
            )),
            Err(rollback) => Err(format!(
                "updater.install: relaunch helper failed ({error}); rollback failed ({rollback}); backup remains at {}",
                backup.display()
            )),
        };
    }

    INSTALL_DONE.store(true, Ordering::Release);
    if relaunch {
        crate::api::app::schedule_exit(std::time::Duration::from_millis(100), 0)
            .map_err(|error| format!("updater.install: exit scheduler: {error}"))?;
    }
    Ok(json!({
        "installed": true,
        "path": current_app.to_string_lossy(),
        "relaunch": relaunch,
    }))
}

#[cfg(target_os = "macos")]
fn macos_bundle_id(app: &Path) -> Result<String, String> {
    let plist = app.join("Contents").join("Info.plist");
    let output = std::process::Command::new("/usr/bin/plutil")
        .args(["-extract", "CFBundleIdentifier", "raw", "-o", "-"])
        .arg(&plist)
        .output()
        .map_err(|e| format!("updater.install: read bundle identifier: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "updater.install: cannot read CFBundleIdentifier from {}",
            plist.display()
        ));
    }
    let identifier = String::from_utf8(output.stdout)
        .map_err(|e| format!("updater.install: bundle identifier is not UTF-8: {e}"))?
        .trim()
        .to_string();
    if identifier.is_empty() {
        Err("updater.install: empty CFBundleIdentifier".into())
    } else {
        Ok(identifier)
    }
}

#[cfg(target_os = "macos")]
fn spawn_macos_cleanup(current_exe: &Path, backup: &Path, relaunch: bool) -> Result<(), String> {
    // Positional: pid, exe, backup, relaunch flag, then the app arguments
    // forwarded to the relaunched process.
    let script = r#"
pid=$1; exe=$2; backup=$3; relaunch=$4; shift 4
while kill -0 "$pid" 2>/dev/null; do sleep 0.2; done
/bin/rm -rf -- "$backup"
if [ "$relaunch" = "1" ]; then exec "$exe" "$@"; fi
"#;
    std::process::Command::new("/bin/sh")
        .arg("-c")
        .arg(script)
        .arg("tynd-updater")
        .arg(std::process::id().to_string())
        .arg(current_exe)
        .arg(backup)
        .arg(if relaunch { "1" } else { "0" })
        .args(crate::api::app::relaunch_args())
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("spawn cleanup/relaunch helper: {e}"))
}

#[cfg(target_os = "windows")]
fn install_windows(new_path: &str, current: &Path, relaunch: bool) -> Result<Value, String> {
    let current_str = current
        .to_str()
        .ok_or("updater.install: current_exe path is not UTF-8")?;

    // `self-replace` handles the Windows file-lock dance — moves the
    // running .exe out of the way (rename) and atomically swaps the new
    // binary in, with no shell involved.
    self_replace::self_replace(new_path)
        .map_err(|e| format!("updater.install: self_replace: {e}"))?;
    INSTALL_DONE.store(true, Ordering::Release);

    // Spawn the swapped-in current_exe before exiting so the new binary
    // takes over without user re-launch.
    Ok(installed_result(
        current_str,
        relaunch_swapped(current, relaunch),
    ))
}

#[cfg(target_os = "linux")]
fn install_linux(new_path: &str, current: &Path, relaunch: bool) -> Result<Value, String> {
    use std::os::unix::fs::PermissionsExt as _;

    // `self_replace` is a no-op on Linux compared to a plain `rename`,
    // but it normalizes the cross-platform code path and protects against
    // future kernel quirks (e.g. exec-busy on overlay filesystems).
    self_replace::self_replace(new_path)
        .map_err(|e| format!("updater.install: self_replace: {e}"))?;
    INSTALL_DONE.store(true, Ordering::Release);

    if let Ok(meta) = std::fs::metadata(current) {
        let mut perm = meta.permissions();
        perm.set_mode(perm.mode() | 0o755);
        if let Err(e) = std::fs::set_permissions(current, perm) {
            // Best-effort: self_replace already preserved the old mode bits,
            // so visibility is enough — don't unlink the running binary.
            crate::tynd_warn!("updater.install: chmod failed: {e}");
        }
    }

    Ok(installed_result(
        &current.to_string_lossy(),
        relaunch_swapped(current, relaunch),
    ))
}

/// Relaunch requested after a completed swap: `Ok(true)` when the new binary
/// was started and this process will exit, `Ok(false)` when no relaunch was
/// asked for.
#[cfg(any(target_os = "windows", target_os = "linux"))]
fn relaunch_swapped(current: &Path, relaunch: bool) -> Result<bool, String> {
    if !relaunch {
        return Ok(false);
    }
    crate::api::app::relaunch_and_exit(current).map(|()| true)
}

/// The binary is already replaced when this runs, so a relaunch failure is
/// reported in the result (`relaunch: false`, `relaunchError`) and logged,
/// never as an install error: the new version is installed and starts on the
/// next launch.
#[cfg(any(target_os = "windows", target_os = "linux"))]
fn installed_result(path: &str, relaunch: Result<bool, String>) -> Value {
    match relaunch {
        Ok(relaunched) => json!({ "installed": true, "path": path, "relaunch": relaunched }),
        Err(error) => {
            crate::tynd_warn!("updater.install: update installed but relaunch failed: {error}");
            json!({
                "installed": true,
                "path": path,
                "relaunch": false,
                "relaunchError": error,
            })
        },
    }
}

#[cfg(test)]
mod symlink_tests {
    use super::symlink_target_is_relative;

    #[test]
    fn accepts_relative_framework_targets() {
        for target in ["A", "Versions/Current/Headers", "../Versions/A/Foo", "./x"] {
            assert!(symlink_target_is_relative(target), "{target}");
        }
    }

    #[test]
    fn refuses_absolute_empty_or_nul_targets() {
        for target in ["", "/etc/passwd", "a\0b"] {
            assert!(!symlink_target_is_relative(target), "{target:?}");
        }
        #[cfg(windows)]
        assert!(!symlink_target_is_relative("C:\\Windows"));
    }
}

#[cfg(test)]
mod owner_tests {
    use super::*;
    use crate::ipc::owner::with_caller;

    fn stage(owner: &str) -> (String, PathBuf) {
        let path = std::env::temp_dir().join(format!("tynd-owner-test-{owner}.bin"));
        std::fs::write(&path, b"x").unwrap();
        let handle = with_caller(owner, || {
            register_verified_artifact(&path, "hash".into(), "binary")
        })
        .unwrap();
        (handle, path)
    }

    #[test]
    fn artifact_of_another_webview_answers_like_an_unknown_handle() {
        let (handle, path) = stage("up-a");

        assert!(with_caller("up-b", || take_verified_artifact(&handle)).is_none());
        assert!(path.exists(), "a refused take must not consume or delete");

        let taken = with_caller("up-a", || take_verified_artifact(&handle));
        assert!(taken.is_some());
        drop(taken);
        assert!(
            !path.exists(),
            "dropping the artifact removes the staged file"
        );
    }

    #[test]
    fn release_reowns_artifacts_and_refuses_remote_callers() {
        use crate::ipc::owner::with_caller_trust;

        let (handle, path) = stage("up-d");
        release_owner("up-d");

        assert!(with_caller_trust("up-d", false, || take_verified_artifact(&handle)).is_none());
        assert!(path.exists());
        assert!(with_caller_trust("up-d", true, || take_verified_artifact(&handle)).is_some());
    }

    #[test]
    fn artifact_staged_after_the_document_was_replaced_is_backend_owned() {
        use crate::ipc::owner::{bump_generation, with_caller_trust};

        let label = "up-late";
        let path = std::env::temp_dir().join(format!("tynd-owner-test-{label}.bin"));
        std::fs::write(&path, b"x").unwrap();
        let handle = with_caller(label, || {
            // The commit lands while the stage call is still running.
            bump_generation(label);
            register_verified_artifact(&path, "hash".into(), "binary")
        })
        .unwrap();

        assert!(with_caller_trust(label, false, || take_verified_artifact(&handle)).is_none());
        assert!(with_caller_trust(label, true, || take_verified_artifact(&handle)).is_some());
    }

    #[test]
    fn backend_may_take_a_webview_artifact() {
        let (handle, _path) = stage("up-c");
        assert!(take_verified_artifact(&handle).is_some());
    }
}

#[cfg(all(test, any(target_os = "windows", target_os = "linux")))]
mod relaunch_result_tests {
    use super::*;

    #[test]
    fn relaunch_failure_after_swap_still_reports_installed() {
        let result = installed_result("/app/bin", Err("spawn: denied".into()));
        assert_eq!(result["installed"], true);
        assert_eq!(result["relaunch"], false);
        assert_eq!(result["relaunchError"], "spawn: denied");
    }

    #[test]
    fn successful_or_skipped_relaunch_has_no_error() {
        let result = installed_result("/app/bin", Ok(true));
        assert_eq!(result["relaunch"], true);
        assert!(result.get("relaunchError").is_none());
        assert_eq!(installed_result("/app/bin", Ok(false))["relaunch"], false);
    }

    #[test]
    fn writable_install_dir_is_self_updatable() {
        // The test binary lives in the writable cargo target dir.
        ensure_self_updatable("downloadAndVerify").expect("target dir is writable");
    }
}
