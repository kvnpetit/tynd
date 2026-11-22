//! System-wide keyboard shortcuts via `global-hotkey`. Fires even when
//! the app is unfocused; emits `shortcut:triggered { id }` with the
//! caller's string id so one listener can multiplex many shortcuts.
//!
//! The OS event handler is installed lazily on first `register` so this
//! module costs nothing at startup when no shortcut is wired.

use std::{
    str::FromStr as _,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, LazyLock, OnceLock,
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use dashmap::{mapref::entry::Entry, DashMap};
use global_hotkey::{hotkey::HotKey, GlobalHotKeyManager};
use parking_lot::Mutex;
use serde_json::{json, Value};

use crate::ipc::error::IpcError;

use crate::api::HostApi;
use crate::ipc::owner::{self, Owner};
use crate::tynd;

/// Build the `shortcuts.<method>: <msg>` IPC error string.
#[cold]
fn shortcut_err(method: &str, msg: impl std::fmt::Display) -> String {
    IpcError::new("shortcuts", method, msg).into_string()
}

/// Outer `OnceLock` caches the init outcome — success or the error string —
/// so the atomic `get_or_init` guarantees exactly one `GlobalHotKeyManager`
/// is constructed even under concurrent `register()` calls.
static MANAGER: OnceLock<Result<Mutex<GlobalHotKeyManager>, String>> = OnceLock::new();

/// `HotKey::id()` → user-supplied string id (or the accelerator string
/// when no explicit id was provided). Keyed by the OS hotkey id so the
/// event handler can route fast.
static ID_MAP: LazyLock<DashMap<u32, String>> = LazyLock::new(DashMap::new);
/// User-supplied string id → registered HotKey. Lets `unregister`
/// recover the HotKey without reparsing.
static HOTKEYS: LazyLock<DashMap<String, Registered>> = LazyLock::new(DashMap::new);

/// A registered hotkey and the webview identity that registered it (`None` for
/// the backend). Only that webview may query or remove it, and it alone receives
/// the trigger event.
#[derive(Clone)]
struct Registered {
    hotkey: HotKey,
    owner: Option<Owner>,
}

impl Registered {
    fn permitted(&self) -> bool {
        owner::permits(self.owner.as_ref())
    }
}
static HANDLER_INSTALLED: AtomicBool = AtomicBool::new(false);
static SHUTTING_DOWN: AtomicBool = AtomicBool::new(false);
static MUTATION: Mutex<()> = Mutex::new(());

const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);

/// Dispatch marker — hotkey state in `MANAGER` / `ID_MAP` / `HOTKEYS`.
#[derive(Debug, Default)]
pub(crate) struct ShortcutsApi;

/// Row of the host API table (`api::HOST_APIS`).
pub(crate) const API: HostApi = HostApi {
    shutdown: Some(shutdown),
    release_webview: Some(release_webview),
    ..HostApi::new("shortcuts", |method, args| dispatch(method, &args))
};

#[tynd::handler(api = "shortcuts")]
impl ShortcutsApi {
    /// Register `accelerator` (e.g. `"Ctrl+Shift+P"`) under `id`
    /// (defaults to the accelerator string). Returns `{ id }` —
    /// listeners on `shortcut:triggered` filter by it.
    pub fn register(accelerator: String, id: Option<String>) -> Result<Value, String> {
        register_impl(accelerator, id)
    }

    /// Unregister the hotkey with the given `id`. Returns `true`
    /// when an entry was removed, `false` when no matching id was
    /// registered.
    pub fn unregister(id: String) -> Result<bool, String> {
        unregister_impl(&id)
    }

    /// Unregister every hotkey currently held. Returns null on
    /// success; surfaces the first per-id failure if any occurred.
    pub fn unregister_all() -> Result<(), String> {
        unregister_all_impl()
    }

    /// True when `id` is currently registered.
    pub fn is_registered(id: String) -> bool {
        !SHUTTING_DOWN.load(Ordering::Acquire)
            && HOTKEYS.get(&id).is_some_and(|entry| entry.permitted())
    }
}

fn ensure_running(operation: &'static str) -> Result<(), String> {
    if SHUTTING_DOWN.load(Ordering::Acquire) {
        Err(shortcut_err(operation, "shortcut service is shutting down"))
    } else {
        Ok(())
    }
}

fn manager() -> Result<&'static Mutex<GlobalHotKeyManager>, String> {
    MANAGER
        .get_or_init(|| {
            GlobalHotKeyManager::new()
                .map(Mutex::new)
                .map_err(|e| shortcut_err("init", format!("GlobalHotKeyManager init failed: {e}")))
        })
        .as_ref()
        .map_err(Clone::clone)
}

/// Idempotent installer for the OS-level hotkey event handler. Routes
/// each `Pressed` event into `crate::ipc::events::emit("shortcut:triggered", ...)`.
fn ensure_handler() {
    if HANDLER_INSTALLED.swap(true, Ordering::AcqRel) {
        return;
    }
    global_hotkey::GlobalHotKeyEvent::set_event_handler(Some(
        |evt: global_hotkey::GlobalHotKeyEvent| {
            if SHUTTING_DOWN.load(Ordering::Acquire) {
                return;
            }
            if !matches!(evt.state, global_hotkey::HotKeyState::Pressed) {
                return;
            }
            let id = ID_MAP
                .get(&evt.id)
                .map_or_else(|| format!("hotkey-{}", evt.id), |e| e.clone());
            let owner = HOTKEYS.get(&id).and_then(|entry| entry.owner.clone());
            owner::emit_for(owner.as_ref(), "shortcut:triggered", &json!({ "id": id }));
        },
    ));
}

fn register_impl(accel: String, id: Option<String>) -> Result<Value, String> {
    ensure_running("register")?;
    let user_id = registration_id(&accel, id);
    let hotkey = parse_hotkey(&accel)?;
    let _mutation = MUTATION.lock();
    ensure_running("register")?;

    ensure_handler();

    match HOTKEYS.entry(user_id.clone()) {
        Entry::Occupied(_) => {
            return Err(shortcut_err(
                "register",
                format!("id '{user_id}' already registered"),
            ));
        },
        Entry::Vacant(slot) => {
            manager()?
                .lock()
                .register(hotkey)
                .map_err(|e| shortcut_err("register", format!("({accel}): {e}")))?;
            if SHUTTING_DOWN.load(Ordering::Acquire) {
                let _ = manager()?.lock().unregister(hotkey);
                return Err(shortcut_err(
                    "register",
                    "shortcut service is shutting down",
                ));
            }
            // A call still running when its webview committed a new document
            // registers after the release: nothing may be left to the next one.
            // `release_webview` takes MUTATION after bumping the generation, so
            // a registration that passes this check is seen by that release.
            let owner = owner::current();
            if owner.as_ref().is_some_and(|owner| !owner.is_current()) {
                let _ = manager()?.lock().unregister(hotkey);
                return Err(shortcut_err(
                    "register",
                    "the document that requested it is gone",
                ));
            }
            ID_MAP.insert(hotkey.id(), user_id.clone());
            slot.insert(Registered { hotkey, owner });
        },
    }

    Ok(json!({ "id": user_id }))
}

fn registration_id(accel: &str, id: Option<String>) -> String {
    id.unwrap_or_else(|| accel.to_string())
}

fn parse_hotkey(accel: &str) -> Result<HotKey, String> {
    HotKey::from_str(accel)
        .map_err(|e| shortcut_err("register", format!("bad accelerator {accel:?}: {e}")))
}

fn unregister_impl(user_id: &str) -> Result<bool, String> {
    ensure_running("unregister")?;
    let _mutation = MUTATION.lock();
    ensure_running("unregister")?;
    unregister_impl_locked(user_id)
}

fn unregister_impl_locked(user_id: &str) -> Result<bool, String> {
    let Some(hotkey) = HOTKEYS
        .get(user_id)
        .filter(|entry| entry.permitted())
        .map(|entry| entry.hotkey)
    else {
        return Ok(false);
    };
    // Try the OS unregister *before* mutating local state. A failure
    // would otherwise leak the OS-level registration while the maps say
    // it's gone — the shortcut keeps firing with no way to recover.
    manager()?
        .lock()
        .unregister(hotkey)
        .map_err(|e| shortcut_err("unregister", format!("({user_id}): {e}")))?;
    HOTKEYS.remove(user_id);
    ID_MAP.remove(&hotkey.id());
    Ok(true)
}

fn unregister_all_impl() -> Result<(), String> {
    ensure_running("unregisterAll")?;
    let _mutation = MUTATION.lock();
    ensure_running("unregisterAll")?;
    unregister_all_locked()
}

/// Unregister the hotkeys `label` registered (its JS is gone).
pub(crate) fn release_webview(label: &str) {
    if SHUTTING_DOWN.load(Ordering::Acquire) {
        return;
    }
    let _mutation = MUTATION.lock();
    let ids: Vec<String> = HOTKEYS
        .iter()
        .filter(|entry| {
            entry
                .owner
                .as_ref()
                .is_some_and(|owner| owner.label() == label)
        })
        .map(|entry| entry.key().clone())
        .collect();
    for id in ids {
        let _ = unregister_impl_locked(&id);
    }
}

/// Unregister every hotkey the caller may operate on: all of them for the
/// backend, only its own for a webview.
fn unregister_all_locked() -> Result<(), String> {
    let ids: Vec<String> = HOTKEYS
        .iter()
        .filter(|entry| entry.permitted())
        .map(|entry| entry.key().clone())
        .collect();
    let mut err: Option<String> = None;
    for id in &ids {
        if let Err(e) = unregister_impl_locked(id) {
            err.get_or_insert(e);
        }
    }
    err.map_or(Ok(()), Err)
}

/// Stop accepting shortcut mutations and release registered OS hotkeys.
///
/// Cleanup runs on a dedicated worker so host shutdown stays bounded even
/// when an OS backend stalls. The global-hotkey callback is process-global
/// and backed by an immutable OnceCell, so it remains installed but becomes
/// inert as soon as `SHUTTING_DOWN` is set.
pub(crate) fn shutdown() {
    let _ = shutdown_with_timeout(SHUTDOWN_TIMEOUT);
}

fn shutdown_with_timeout(timeout: Duration) -> Option<JoinHandle<()>> {
    if SHUTTING_DOWN.swap(true, Ordering::AcqRel) {
        return None;
    }

    let (done_tx, done_rx) = mpsc::sync_channel(1);
    let worker = match thread::Builder::new()
        .name("tynd-shortcuts-shutdown".to_string())
        .spawn(move || {
            let result = {
                let _mutation = MUTATION.lock();
                unregister_all_locked()
            };
            HOTKEYS.clear();
            ID_MAP.clear();
            let _ = done_tx.send(result);
        }) {
        Ok(worker) => worker,
        Err(error) => {
            HOTKEYS.clear();
            ID_MAP.clear();
            tracing::warn!(
                target: "tynd::cleanup",
                %error,
                "failed to start global-shortcut cleanup worker"
            );
            return None;
        },
    };

    match done_rx.recv_timeout(timeout) {
        Ok(result) => {
            if let Err(error) = result {
                tracing::warn!(
                    target: "tynd::cleanup",
                    %error,
                    "failed to unregister one or more global shortcuts"
                );
            }
            if worker.join().is_err() {
                tracing::warn!(
                    target: "tynd::cleanup",
                    "global-shortcut cleanup worker panicked"
                );
            }
            None
        },
        Err(mpsc::RecvTimeoutError::Timeout) => {
            tracing::warn!(
                target: "tynd::cleanup",
                timeout_ms = timeout.as_millis(),
                "global-shortcut cleanup timed out; worker detached"
            );
            Some(worker)
        },
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            HOTKEYS.clear();
            ID_MAP.clear();
            if worker.join().is_err() {
                tracing::warn!(
                    target: "tynd::cleanup",
                    "global-shortcut cleanup worker panicked"
                );
            }
            None
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ipc::owner::with_caller;

    fn insert(id: &str, owner: Option<&str>) {
        HOTKEYS.insert(
            id.to_string(),
            Registered {
                hotkey: HotKey::from_str("Ctrl+Shift+F12").expect("hotkey"),
                owner: owner.and_then(|label| with_caller(label, owner::current)),
            },
        );
    }

    #[test]
    fn another_webviews_shortcut_answers_like_an_unknown_id() {
        insert("sc-owned", Some("sc-a"));
        let query = |caller: &'static str, id: &'static str| {
            with_caller(caller, || ShortcutsApi::is_registered(id.to_string()))
        };
        assert!(query("sc-a", "sc-owned"));
        assert_eq!(query("sc-b", "sc-owned"), query("sc-b", "sc-missing"));
        assert!(!query("sc-b", "sc-owned"));
        // Refused unregister never reaches the OS manager and reports `false`.
        assert!(!with_caller("sc-b", || unregister_impl_locked("sc-owned")).unwrap());
        assert!(HOTKEYS.contains_key("sc-owned"));
        // The backend reaches every shortcut.
        assert!(ShortcutsApi::is_registered("sc-owned".to_string()));
        HOTKEYS.remove("sc-owned");
    }
}
