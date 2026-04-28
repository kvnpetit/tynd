//! Cross-OS single-instance lock + argv forwarding.
//!
//! The first launch acquires an exclusive `flock`/`LockFileEx` on a file
//! under `dirs::cache_dir()/tynd/<id>.lock` (`fslock`). It also spawns a
//! listener on a platform-local socket (named pipe on Windows, Unix socket
//! elsewhere) via `interprocess`. A second launch fails to take the file
//! lock, connects to the socket instead, sends its argv + cwd, and exits —
//! so the user sees the existing instance focus instead of a new window.
//!
//! Migrated from the unmaintained `single-instance` crate to `fslock` —
//! file-based, self-cleaning on process exit, no abstract socket trickery.

use fslock::LockFile;
use interprocess::local_socket::{prelude::*, GenericNamespaced, ListenerOptions, Stream};
use parking_lot::Mutex;
use serde_json::{json, Value};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::sync::OnceLock;

use super::events;

// File lock outlives the process; keep it in a static so the OS releases
// only on real exit.
static LOCK: OnceLock<Mutex<Option<LockFile>>> = OnceLock::new();

/// Rust-side callback fired alongside the `app:second-launch` OS event —
/// lets `app.rs` refocus the primary window without going through the
/// webview round-trip.
pub type SecondLaunchHook = Box<dyn Fn(&Value) + Send + Sync + 'static>;
static SECOND_LAUNCH_HOOK: OnceLock<SecondLaunchHook> = OnceLock::new();

pub fn set_second_launch_hook(f: SecondLaunchHook) {
    let _ = SECOND_LAUNCH_HOOK.set(f);
}

pub fn dispatch(method: &str, args: &Value) -> Result<Value, String> {
    match method {
        "acquire" => acquire(args),
        "isAcquired" => Ok(Value::Bool(is_held())),
        _ => Err(format!("singleInstance.{method}: unknown method")),
    }
}

fn lock_path(id: &str) -> Result<PathBuf, String> {
    let base = dirs::cache_dir()
        .or_else(dirs::data_dir)
        .ok_or_else(|| "singleInstance: no cache/data dir available".to_string())?;
    let dir = base.join("tynd");
    fs::create_dir_all(&dir).map_err(|e| format!("singleInstance: mkdir {}: {e}", dir.display()))?;
    Ok(dir.join(format!("{id}.lock")))
}

fn acquire(args: &Value) -> Result<Value, String> {
    let id = args
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| "singleInstance.acquire: missing 'id'".to_string())?;

    let cell = LOCK.get_or_init(|| Mutex::new(None));
    let mut guard = cell.lock();
    if guard.is_some() {
        return Ok(json!({ "acquired": true, "already": true }));
    }
    let path = lock_path(id)?;
    let mut lock = LockFile::open(&path)
        .map_err(|e| format!("singleInstance.acquire: open {}: {e}", path.display()))?;
    let acquired = lock
        .try_lock()
        .map_err(|e| format!("singleInstance.acquire: try_lock: {e}"))?;
    if acquired {
        *guard = Some(lock);
        spawn_forward_listener(id);
    } else {
        // Lock held by another process — forward our argv/cwd to it and
        // let the caller decide (typically: exit with code 0).
        let _ = forward_to_primary(id);
    }
    Ok(json!({ "acquired": acquired, "already": false }))
}

fn is_held() -> bool {
    LOCK.get().is_some_and(|c| c.lock().is_some())
}

/// Cross-platform local-socket name derived from the user-supplied id.
/// Windows: `\\.\pipe\tynd-<id>` · Unix: abstract / path-based socket.
///
/// `Box::leak` the formatted string once per call. Acquire is invoked at
/// most twice per process lifetime (listener + forwarder), so the leak is
/// bounded to a few dozen bytes. Earlier OnceLock-cached versions silently
/// returned a stale name when called with a different `id`.
fn socket_name(id: &str) -> Result<interprocess::local_socket::Name<'static>, String> {
    let leaked: &'static str = Box::leak(format!("tynd-{id}").into_boxed_str());
    leaked
        .to_ns_name::<GenericNamespaced>()
        .map_err(|e| format!("socket name: {e}"))
}

/// Accept incoming connections on the local socket and emit
/// `app:second-launch { argv, cwd }` for each forwarded payload.
fn spawn_forward_listener(id: &str) {
    let name = match socket_name(id) {
        Ok(n) => n,
        Err(e) => {
            crate::tynd_log!("singleInstance listener: {e}");
            return;
        },
    };

    let listener = match ListenerOptions::new().name(name).create_sync() {
        Ok(l) => l,
        Err(e) => {
            crate::tynd_log!("singleInstance listener: {e}");
            return;
        },
    };

    std::thread::Builder::new()
        .name("tynd-singleinstance".into())
        .spawn(move || {
            for conn in listener.incoming() {
                let conn = match conn {
                    Ok(c) => c,
                    Err(e) => {
                        crate::tynd_log!("singleInstance accept: {e}");
                        continue;
                    },
                };
                let mut reader = BufReader::new(conn);
                let mut line = String::new();
                if reader.read_line(&mut line).is_err() || line.trim().is_empty() {
                    continue;
                }
                let Ok(payload) = serde_json::from_str::<Value>(line.trim()) else {
                    continue;
                };
                events::emit("app:second-launch", &payload);
                if let Some(hook) = SECOND_LAUNCH_HOOK.get() {
                    hook(&payload);
                }
            }
        })
        .ok();
}

/// 2nd-launch path: connect to the running instance's socket and send
/// `{argv, cwd}` as a single JSON line. Silently ignored on failure —
/// worst case the duplicate just runs standalone.
fn forward_to_primary(id: &str) -> Result<(), String> {
    let name = socket_name(id)?;
    let mut stream = Stream::connect(name).map_err(|e| format!("connect: {e}"))?;
    let argv: Vec<String> = std::env::args().collect();
    let cwd = std::env::current_dir()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    let payload = json!({ "argv": argv, "cwd": cwd });
    let mut line = serde_json::to_string(&payload).unwrap_or_default();
    line.push('\n');
    stream
        .write_all(line.as_bytes())
        .map_err(|e| format!("write: {e}"))?;
    Ok(())
}
