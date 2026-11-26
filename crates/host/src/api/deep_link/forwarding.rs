//! Multi-instance URL forwarding for the deep-link subsystem.
//!
//! When the OS hands the user a `myapp://...` link and your app is
//! **already running**, every supported platform spawns a *fresh*
//! process that receives the URL via `argv[1..]` and (typically) exits
//! immediately. Without coordination the URL is lost — the original
//! process never sees it.
//!
//! Mirrors the pattern from `api::single_instance` but with an
//! independent lock + socket name so the two systems coexist (an app
//! can opt into both — see `api::single_instance::set_second_launch_hook`).
//!
//! 1. The first launch acquires an exclusive `flock`/`LockFileEx` on a file under
//!    `dirs::cache_dir()/tynd/<app_id>-deeplink.lock` (`fslock`). It then spawns a listener thread
//!    on a local socket (Windows named pipe / Unix domain socket via `interprocess`).
//! 2. Subsequent OS-spawned launches that carry a fresh URL fail to take the file lock, connect to
//!    the socket, write the URL as a single newline-terminated `URL:<url>\n` line, and return. The
//!    caller's `init` flips `forwarded_to_primary = true` so the host bootstrap can
//!    `process::exit(0)` without ever spawning a webview.
//! 3. The listener forwards every received URL to the host's event bus as `deep-link:url { url }` —
//!    the same shape the macOS Apple-Event handler emits.

use std::{io::Write, path::PathBuf};

use fslock::LockFile;
use interprocess::local_socket::ListenerOptions;
use parking_lot::Mutex;

#[derive(Debug)]
pub(super) enum AcquireOutcome {
    /// We are the master instance — keep this guard alive for the rest
    /// of the process to hold the OS lock.
    Master(MasterGuard),
    /// Another process is already master. The URL (if any) was
    /// forwarded to it; the caller should exit cleanly.
    Forwarded,
}

/// RAII guard around the master-instance file lock. Holding it keeps
/// the lock alive for the rest of the process lifetime.
#[derive(Debug)]
pub(super) struct MasterGuard {
    _lock: Mutex<Option<LockFile>>,
}

/// Try to acquire the per-app deep-link master lock. If already held,
/// forward `startup_url` (when present) to the master via the local
/// socket and return [`AcquireOutcome::Forwarded`].
pub(super) fn acquire_or_forward(
    app_id: &str,
    startup_url: Option<&str>,
) -> Result<AcquireOutcome, String> {
    let lock_path = lock_path(app_id)?;
    let mut lock = LockFile::open(&lock_path)
        .map_err(|e| format!("deep-link forwarding: open {}: {e}", lock_path.display()))?;
    let acquired = lock
        .try_lock()
        .map_err(|e| format!("deep-link forwarding: try_lock: {e}"))?;
    if acquired {
        Ok(AcquireOutcome::Master(MasterGuard {
            _lock: Mutex::new(Some(lock)),
        }))
    } else {
        if let Some(u) = startup_url {
            forward_url_to_master(app_id, u)?;
        }
        Ok(AcquireOutcome::Forwarded)
    }
}

/// Spawn the master-side listener thread that re-emits URLs forwarded
/// by second-launch processes onto the host event bus.
pub(super) fn spawn_listener(
    app_id: &str,
) -> Result<crate::platform::local_listener::LocalListener, String> {
    let name = socket_name(app_id)
        .map_err(|error| format!("deep-link forwarding: socket name: {error}"))?;
    // The master lock is held, so a leftover socket file is from a crash.
    crate::platform::local_listener::remove_stale_endpoint("deeplink", &socket_key(app_id));
    let listener_options = ListenerOptions::new().name(name);
    #[cfg(windows)]
    let listener_options = apply_owner_only_dacl(listener_options)?;
    let listener = listener_options
        .create_sync()
        .map_err(|error| format!("deep-link forwarding: bind listener: {error}"))?;

    crate::platform::local_listener::spawn("tynd-deeplink-listener", listener, |line| {
        let Some(url) = line.trim().strip_prefix("URL:") else {
            return;
        };
        if let Some(url) = super::parse_custom_scheme_url(url) {
            super::emit_url(&url);
        }
    })
}

const MAX_FORWARD_BYTES: usize = 64 * 1024;

fn lock_path(app_id: &str) -> Result<PathBuf, String> {
    let base = dirs::cache_dir()
        .or_else(dirs::data_dir)
        .ok_or_else(|| "deep-link forwarding: no cache/data dir available".to_string())?;
    let dir = base.join("tynd");
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("deep-link forwarding: mkdir {}: {e}", dir.display()))?;
    Ok(dir.join(format!("{}-deeplink.lock", sanitize(app_id))))
}

/// Longest sanitized app id kept verbatim in the socket key; longer ids are
/// replaced by a BLAKE3 prefix so the name stays bounded (named-pipe and
/// `sun_path` limits) on every platform.
const SOCKET_KEY_BYTE_CAP: usize = 64;

fn socket_key(app_id: &str) -> String {
    let sanitized = sanitize(app_id);
    if sanitized.len() > SOCKET_KEY_BYTE_CAP {
        // 64 bits of BLAKE3 are ample for an app-local namespace.
        let hash = blake3::hash(sanitized.as_bytes());
        hash.to_hex().as_str()[..16].to_string()
    } else {
        sanitized
    }
}

/// Per-user endpoint (see `crate::platform::local_listener::endpoint_name`): owner-only
/// pipe on Windows, a socket inside a private `0700` directory on Unix.
fn socket_name(app_id: &str) -> Result<interprocess::local_socket::Name<'static>, String> {
    crate::platform::local_listener::endpoint_name("deeplink", &socket_key(app_id))
}

fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// See `api::single_instance::apply_owner_only_dacl` — same rationale,
/// applied to the deep-link forwarding pipe. A permissive DACL would
/// let any local user push a `URL:javascript:...` payload into the
/// master process's `deep-link:url` event bus.
#[cfg(windows)]
fn apply_owner_only_dacl(opts: ListenerOptions<'_>) -> Result<ListenerOptions<'_>, String> {
    use interprocess::os::windows::local_socket::ListenerOptionsExt;

    crate::platform::win_security::current_user_sd()
        .map(|descriptor| opts.security_descriptor(descriptor))
        .map_err(|error| format!("deep-link forwarding: owner-only DACL failed: {error}"))
}

fn forward_url_to_master(app_id: &str, url: &str) -> Result<(), String> {
    if url.contains(['\r', '\n']) {
        return Err("forward URL contains a line break".into());
    }
    let mut stream =
        crate::platform::local_listener::connect_endpoint("deeplink", &socket_key(app_id))?;
    crate::platform::local_listener::verify_server_is_current_user(&stream)?;
    let mut line = format!("URL:{url}");
    line.push('\n');
    if line.len() > MAX_FORWARD_BYTES {
        return Err(format!(
            "forward URL exceeds the {MAX_FORWARD_BYTES}-byte cap"
        ));
    }
    stream
        .write_all(line.as_bytes())
        .map_err(|e| format!("write: {e}"))?;
    Ok(())
}
