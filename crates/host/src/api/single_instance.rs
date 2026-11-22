//! Cross-OS single-instance lock + argv forwarding.
//!
//! First launch acquires an exclusive `fslock` on a file in `cache_dir/tynd/`
//! and spawns a listener on a platform-local socket. A second launch fails the
//! file lock, connects to the socket, ships argv + cwd, and exits — the
//! existing instance focuses instead.

use std::{
    fs,
    io::Write,
    path::PathBuf,
    sync::{LazyLock, OnceLock},
};

use fslock::LockFile;
use interprocess::local_socket::ListenerOptions;
use parking_lot::Mutex;
use serde_json::{json, Value};

use crate::ipc::error::IpcError;

use crate::api::HostApi;
use crate::ipc::events;
use crate::tynd;

#[derive(Default)]
struct SingleInstanceState {
    lock: Option<LockFile>,
    listener: Option<crate::platform::local_listener::LocalListener>,
    /// Id the lock was acquired for, kept so a failed relaunch can re-acquire.
    id: Option<String>,
    shutting_down: bool,
}

static STATE: LazyLock<Mutex<SingleInstanceState>> =
    LazyLock::new(|| Mutex::new(SingleInstanceState::default()));

/// Rust-side callback fired alongside the `app:second-launch` OS event —
/// lets the app event loop refocus the primary window without going through the
/// webview round-trip.
pub(crate) type SecondLaunchHook = Box<dyn Fn(&Value) + Send + Sync + 'static>;
static SECOND_LAUNCH_HOOK: OnceLock<SecondLaunchHook> = OnceLock::new();

pub(crate) fn set_second_launch_hook(f: SecondLaunchHook) {
    let _ = SECOND_LAUNCH_HOOK.set(f);
}

/// Dispatch marker — state lives in `STATE`.
#[derive(Debug, Default)]
pub(crate) struct SingleInstanceApi;

/// Row of the host API table (`api::HOST_APIS`).
pub(crate) const API: HostApi = HostApi {
    shutdown: Some(shutdown),
    ..HostApi::new("singleInstance", |method, args| dispatch(method, &args))
};

#[tynd::handler(api = "singleInstance")]
impl SingleInstanceApi {
    /// Try to acquire the exclusive lock for `id`. Returns
    /// `{ acquired, already }` — `acquired` is `true` when this is the
    /// primary instance and `false` when another instance already holds
    /// the lock (in which case argv+cwd are forwarded to the holder).
    /// `already` is `true` when this process already holds the lock
    /// (idempotent re-call).
    pub fn acquire(id: String) -> Result<Value, String> {
        acquire_impl(&id)
    }

    /// True when this process currently holds the single-instance
    /// lock for any id.
    pub fn is_acquired() -> bool {
        is_held()
    }
}

fn single_instance_base_dir() -> Result<PathBuf, String> {
    dirs::cache_dir().or_else(dirs::data_dir).ok_or_else(|| {
        IpcError::new("singleInstance", "lock_path", "no cache/data dir available").into_string()
    })
}

fn lock_path(id: &str) -> Result<PathBuf, String> {
    validate_instance_id(id)?;
    let dir = single_instance_base_dir()?.join("tynd");
    fs::create_dir_all(&dir)
        .map_err(|e| IpcError::new("singleInstance", "lock_path", e).into_string())?;
    Ok(dir.join(format!("{id}.lock")))
}

fn acquire_impl(id: &str) -> Result<Value, String> {
    let mut state = STATE.lock();
    if state.shutting_down {
        return Err("singleInstance.acquire: host is shutting down".into());
    }
    if state.lock.is_some() {
        return Ok(json!({ "acquired": true, "already": true }));
    }
    let path = lock_path(id)?;
    let mut lock = LockFile::open(&path).map_err(|e| {
        IpcError::new(
            "singleInstance",
            "acquire",
            format!("open {}: {e}", path.display()),
        )
        .into_string()
    })?;
    let acquired = lock.try_lock().map_err(|e| {
        IpcError::new("singleInstance", "acquire", format!("try_lock: {e}")).into_string()
    })?;
    if acquired {
        match spawn_forward_listener(id) {
            Ok(listener) => {
                state.listener = Some(listener);
                state.lock = Some(lock);
                state.id = Some(id.to_string());
            },
            // The forwarding endpoint is unavailable (for example another
            // account squatted the pipe name). Start anyway without
            // single-instance enforcement: keeping the lock without a listener
            // would make every later launch fail to forward.
            Err(error) => {
                crate::tynd_warn!("single-instance forwarding disabled: {error}");
            },
        }
    } else {
        // Lock held by another process — forward our argv/cwd to it and
        // let the caller decide (typically: exit with code 0).
        forward_to_primary(id)?;
    }
    Ok(json!({ "acquired": acquired, "already": false }))
}

fn is_held() -> bool {
    STATE.lock().lock.is_some()
}

/// Cross-platform local-socket name derived from the user-supplied id.
/// Windows: `\\.\pipe\tynd-single-instance-<id>` (owner-only DACL) · Unix: a socket file in
/// a per-user `0700` directory (see `local_listener::endpoint_name`).
fn socket_name(id: &str) -> Result<interprocess::local_socket::Name<'static>, String> {
    validate_instance_id(id)?;
    crate::platform::local_listener::endpoint_name("single-instance", id)
}

/// Accept incoming connections on the local socket and emit
/// `app:second-launch { argv, cwd }` for each forwarded payload.
fn spawn_forward_listener(
    id: &str,
) -> Result<crate::platform::local_listener::LocalListener, String> {
    let name = socket_name(id).map_err(|error| format!("singleInstance listener: {error}"))?;
    // We hold the instance lock, so a leftover socket file is from a crash.
    crate::platform::local_listener::remove_stale_endpoint("single-instance", id);

    let listener_options = ListenerOptions::new().name(name);
    #[cfg(windows)]
    let listener_options = apply_owner_only_dacl(listener_options)?;
    let listener = listener_options
        .create_sync()
        .map_err(|error| format!("singleInstance listener bind failed: {error}"))?;

    crate::platform::local_listener::spawn("tynd-singleinstance", listener, |line| {
        let Ok(payload) = serde_json::from_str::<Value>(line.trim()) else {
            return;
        };
        events::emit("app:second-launch", &payload);
        if let Some(hook) = SECOND_LAUNCH_HOOK.get() {
            hook(&payload);
        }
    })
}

/// Release the lock and the forwarding listener so a relaunched copy of this
/// app can become the primary instance while this process is still exiting.
/// Returns the released id, to pass to [`restore_after_failed_relaunch`] when
/// the copy could not be started.
pub(crate) fn release_for_relaunch() -> Option<String> {
    let (listener, lock, id) = {
        let mut state = STATE.lock();
        (state.listener.take(), state.lock.take(), state.id.take())
    };
    if let Some(listener) = listener {
        listener.shutdown();
    }
    drop(lock);
    id
}

/// Re-acquire the lock released by [`release_for_relaunch`] after the
/// relaunch failed, so this process stays the primary instance.
pub(crate) fn restore_after_failed_relaunch(id: &str) {
    match acquire_impl(id) {
        Ok(result) if result.get("acquired") == Some(&Value::Bool(true)) => {},
        Ok(_) => crate::tynd_warn!(
            "single-instance lock '{id}' was taken by another process during a failed relaunch"
        ),
        Err(error) => crate::tynd_warn!("single-instance lock '{id}' not re-acquired: {error}"),
    }
}

pub(crate) fn shutdown() {
    let (listener, lock) = {
        let mut state = STATE.lock();
        state.shutting_down = true;
        state.id = None;
        (state.listener.take(), state.lock.take())
    };
    if let Some(listener) = listener {
        listener.shutdown();
    }
    drop(lock);
}

const MAX_FORWARD_BYTES: usize = 64 * 1024;

/// Restrict the named-pipe DACL to the running user only. The default
/// `interprocess` DACL on Windows lets any local user connect — and a
/// connecting peer can ship arbitrary argv/cwd to the existing
/// instance. Building an owner-only SD closes that gap. Failure is fatal
/// for this optional listener: binding a permissive pipe would cross the
/// user boundary and let another account inject argv.
#[cfg(windows)]
fn apply_owner_only_dacl(opts: ListenerOptions<'_>) -> Result<ListenerOptions<'_>, String> {
    use interprocess::os::windows::local_socket::ListenerOptionsExt;

    crate::platform::win_security::current_user_sd()
        .map(|descriptor| opts.security_descriptor(descriptor))
        .map_err(|error| format!("singleInstance owner-only DACL build failed: {error}"))
}

fn validate_instance_id(id: &str) -> Result<(), String> {
    if id.is_empty()
        || id.len() > 128
        || matches!(id, "." | "..")
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err(IpcError::new(
            "singleInstance",
            "acquire",
            "id must contain 1..=128 ASCII letters, digits, '.', '_' or '-'",
        )
        .into_string());
    }
    Ok(())
}

/// 2nd-launch path: connect to the running instance's socket and send
/// `{argv, cwd}` as a single bounded JSON line. Failures propagate so the
/// caller does not report a successful forward and discard launch context.
fn forward_to_primary(id: &str) -> Result<(), String> {
    validate_instance_id(id)?;
    let mut stream = crate::platform::local_listener::connect_endpoint("single-instance", id)?;
    crate::platform::local_listener::verify_server_is_current_user(&stream)?;
    let argv: Vec<String> = std::env::args().collect();
    let cwd = std::env::current_dir()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    let payload = json!({ "argv": argv, "cwd": cwd });
    let mut line =
        serde_json::to_string(&payload).map_err(|error| format!("serialize payload: {error}"))?;
    line.push('\n');
    if line.len() > MAX_FORWARD_BYTES {
        return Err(format!(
            "second-launch payload exceeds the {MAX_FORWARD_BYTES}-byte cap"
        ));
    }
    stream
        .write_all(line.as_bytes())
        .map_err(|e| format!("write: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn unique_id(tag: &str) -> String {
        format!("tynd-test-{tag}-{}", std::process::id())
    }

    #[test]
    fn instance_ids_are_validated() {
        for ok in ["app", "com.example.app", "My_App-1", &"a".repeat(128)] {
            assert!(validate_instance_id(ok).is_ok(), "{ok}");
        }
        for bad in [
            "",
            ".",
            "..",
            "a/b",
            r"a\b",
            "a b",
            "a:b",
            "é",
            &"a".repeat(129),
        ] {
            assert!(validate_instance_id(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn lock_path_is_per_id_and_rejects_bad_ids() {
        let a = lock_path("alpha").unwrap();
        let b = lock_path("beta").unwrap();
        assert_ne!(a, b);
        assert!(a.ends_with("alpha.lock"));
        assert!(lock_path("../escape").is_err());
    }

    #[test]
    fn exclusive_lock_is_refused_while_another_handle_holds_it() {
        let path = lock_path(&unique_id("lock")).unwrap();
        let mut first = LockFile::open(&path).unwrap();
        assert!(first.try_lock().unwrap());
        let mut second = LockFile::open(&path).unwrap();
        assert!(!second.try_lock().unwrap());
        drop(first);
        assert!(second.try_lock().unwrap());
        drop(second);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn second_launch_payload_reaches_the_listener_and_hook() {
        let (tx, rx) = std::sync::mpsc::channel::<Value>();
        set_second_launch_hook(Box::new(move |payload| {
            let _ = tx.send(payload.clone());
        }));
        let id = unique_id("fwd");
        let listener = spawn_forward_listener(&id).expect("bind listener");

        forward_to_primary(&id).expect("forward");
        let payload = rx
            .recv_timeout(Duration::from_secs(5))
            .expect("hook must receive the forwarded payload");
        assert!(payload["argv"].is_array(), "{payload}");
        assert!(payload["cwd"].is_string(), "{payload}");

        listener.shutdown();
        assert!(
            forward_to_primary(&id).is_err(),
            "forwarding to a stopped listener must fail, not silently succeed"
        );
    }

    #[test]
    fn forwarding_without_a_primary_is_an_error() {
        assert!(forward_to_primary(&unique_id("none")).is_err());
    }
}
