//! Cross-OS single-instance lock + argv forwarding.
//!
//! The first launch holds the lock (`single-instance` crate) and spawns a
//! listener on a platform-local socket (named pipe on Windows, Unix socket
//! elsewhere). A second launch finds the lock already held, connects to the
//! socket, sends its argv + cwd, and exits — so the user sees the existing
//! instance focus instead of a new duplicate window.

use interprocess::local_socket::{prelude::*, GenericNamespaced, ListenerOptions, Stream};
use parking_lot::Mutex;
use serde_json::{json, Value};
use single_instance::SingleInstance;
use std::io::{BufRead, BufReader, Write};
use std::sync::OnceLock;

use super::events;

// Lock must outlive the process; keep it in a static so the OS release
// only happens on real exit.
static LOCK: OnceLock<Mutex<Option<SingleInstance>>> = OnceLock::new();

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
    let instance = SingleInstance::new(id).map_err(|e| format!("singleInstance.acquire: {e}"))?;
    let acquired = instance.is_single();
    if acquired {
        *guard = Some(instance);
        // First-instance only: stand up a listener so subsequent launches
        // can forward their argv + cwd here before they exit.
        spawn_forward_listener(id);
    } else {
        // Lock is held by another process — forward our argv/cwd to it and
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
fn socket_name(id: &str) -> Result<interprocess::local_socket::Name<'_>, String> {
    // Return a `Name<'static>`? We borrow the &str so callers must keep it alive.
    // interprocess's GenericNamespaced::to_ns_name is zero-copy on valid input.
    let sanitized = format!("tynd-{id}");
    // `Box::leak` gives a `&'static str` — fine since the socket lives for the
    // process lifetime anyway.
    let leaked: &'static str = Box::leak(sanitized.into_boxed_str());
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
