//! Multi-instance URL forwarding for the deep-link subsystem.
//!
//! When the OS hands the user a `myapp://...` link and your app is
//! **already running**, every supported platform spawns a *fresh*
//! process that receives the URL via `argv[1..]` and (typically) exits
//! immediately. Without coordination the URL is lost — the original
//! process never sees it.
//!
//! Mirrors the pattern from `os::single_instance` but with an
//! independent lock + socket name so the two systems coexist (an app
//! can opt into both — see `os::single_instance::set_second_launch_hook`).
//!
//! 1. The first launch acquires an exclusive `flock`/`LockFileEx` on a
//!    file under `dirs::cache_dir()/tynd/<app_id>-deeplink.lock`
//!    (`fslock`). It then spawns a listener thread on a local socket
//!    (Windows named pipe / Unix domain socket via `interprocess`).
//! 2. Subsequent OS-spawned launches that carry a fresh URL fail to
//!    take the file lock, connect to the socket, write the URL as a
//!    single newline-terminated `URL:<url>\n` line, and return. The
//!    caller's `init` flips `forwarded_to_primary = true` so the host
//!    bootstrap can `process::exit(0)` without ever spawning a webview.
//! 3. The listener forwards every received URL to the host's event bus
//!    as `deep-link:url { url }` — the same shape the macOS Apple-Event
//!    handler emits.

use std::{
    io::{BufRead, BufReader, Write},
    path::PathBuf,
    sync::OnceLock,
};

use fslock::LockFile;
use interprocess::local_socket::{prelude::*, GenericNamespaced, ListenerOptions, Stream};
use parking_lot::Mutex;

#[derive(Debug)]
pub(super) enum AcquireOutcome {
    /// We are the master instance — keep this guard alive for the rest
    /// of the process to hold the OS lock.
    Master(MasterGuard),
    /// Another process is already master. The URL (if any) was
    /// forwarded to it; the caller should exit cleanly.
    #[allow(dead_code)]
    Forwarded { url_sent: Option<String> },
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
        let url_sent = match startup_url {
            Some(u) => match forward_url_to_master(app_id, u) {
                Ok(()) => Some(u.to_string()),
                Err(e) => {
                    crate::tynd_log!("deep-link: URL forward failed: {e}");
                    None
                },
            },
            None => None,
        };
        Ok(AcquireOutcome::Forwarded { url_sent })
    }
}

/// Spawn the master-side listener thread that re-emits URLs forwarded
/// by second-launch processes onto the host event bus.
///
/// Idempotent — calling twice is a no-op (only the first call binds the
/// socket; subsequent calls return early via `LISTENER_BOUND`).
pub(super) fn spawn_listener(app_id: &str) {
    if LISTENER_BOUND.get().is_some() {
        return;
    }

    let name = match socket_name(app_id) {
        Ok(n) => n,
        Err(e) => {
            crate::tynd_log!("deep-link forwarding: socket name: {e}");
            return;
        },
    };
    let listener = match ListenerOptions::new().name(name).create_sync() {
        Ok(l) => l,
        Err(e) => {
            crate::tynd_log!("deep-link forwarding: bind listener: {e}");
            return;
        },
    };
    let _ = LISTENER_BOUND.set(());

    std::thread::Builder::new()
        .name("tynd-deeplink-listener".into())
        .spawn(move || {
            for conn in listener.incoming() {
                let conn = match conn {
                    Ok(c) => c,
                    Err(e) => {
                        crate::tynd_log!("deep-link forwarding: accept: {e}");
                        continue;
                    },
                };
                let mut reader = BufReader::new(conn);
                let mut line = String::new();
                if reader.read_line(&mut line).is_err() || line.trim().is_empty() {
                    continue;
                }
                let trimmed = line.trim();
                let Some(url) = trimmed.strip_prefix("URL:") else {
                    continue;
                };
                super::emit_url(url);
            }
        })
        .ok();
}

/// Marker that the listener thread is already running for this process.
static LISTENER_BOUND: OnceLock<()> = OnceLock::new();

fn lock_path(app_id: &str) -> Result<PathBuf, String> {
    let base = dirs::cache_dir()
        .or_else(dirs::data_dir)
        .ok_or_else(|| "deep-link forwarding: no cache/data dir available".to_string())?;
    let dir = base.join("tynd");
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("deep-link forwarding: mkdir {}: {e}", dir.display()))?;
    Ok(dir.join(format!("{}-deeplink.lock", sanitize(app_id))))
}

fn socket_name(app_id: &str) -> Result<interprocess::local_socket::Name<'static>, String> {
    let leaked: &'static str =
        Box::leak(format!("tynd-deeplink-{}", sanitize(app_id)).into_boxed_str());
    leaked
        .to_ns_name::<GenericNamespaced>()
        .map_err(|e| format!("socket name: {e}"))
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

fn forward_url_to_master(app_id: &str, url: &str) -> Result<(), String> {
    let name = socket_name(app_id)?;
    let mut stream = Stream::connect(name).map_err(|e| format!("connect: {e}"))?;
    let mut line = format!("URL:{url}");
    line.push('\n');
    stream
        .write_all(line.as_bytes())
        .map_err(|e| format!("write: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_rewrites_unsafe_chars() {
        assert_eq!(sanitize("com.example.myapp"), "com.example.myapp");
        assert_eq!(sanitize("foo/bar:baz"), "foo_bar_baz");
        assert_eq!(sanitize("a b c"), "a_b_c");
        assert_eq!(sanitize(""), "");
    }

    #[test]
    fn lock_path_uses_app_id_suffix() {
        let path = lock_path("com.example.myapp").expect("cache dir present");
        let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
        assert!(name.ends_with("-deeplink.lock"), "got: {name}");
        assert!(name.contains("com.example.myapp"), "got: {name}");
    }

    #[test]
    fn acquire_or_forward_returns_master_first_time() {
        let app_id = format!("test-{}", std::process::id());
        let outcome = acquire_or_forward(&app_id, None);
        match outcome {
            Ok(AcquireOutcome::Master(_)) | Ok(AcquireOutcome::Forwarded { .. }) | Err(_) => {},
        }
    }
}
