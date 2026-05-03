//! System-wide keyboard shortcuts via `global-hotkey`. Fires even when
//! the app is unfocused; emits `shortcut:triggered { id }` with the
//! caller's string id so one listener can multiplex many shortcuts.
//!
//! Promoted from `@tynd/plugin-shortcuts`. The OS event handler is
//! installed lazily on first `register` so this module costs nothing at
//! startup when no shortcut is wired.

use std::{
    str::FromStr as _,
    sync::{
        atomic::{AtomicBool, Ordering},
        LazyLock, OnceLock,
    },
};

use dashmap::DashMap;
use global_hotkey::{hotkey::HotKey, GlobalHotKeyManager};
use parking_lot::Mutex;
use serde_json::{json, Value};

use super::events;
use crate::{ipc_error::IpcError, tynd};

/// Outer `OnceLock` caches the init outcome — success or the error string —
/// so the atomic `get_or_init` guarantees exactly one `GlobalHotKeyManager`
/// is constructed even under concurrent `register()` calls.
static MANAGER: OnceLock<Result<Mutex<GlobalHotKeyManager>, String>> = OnceLock::new();

/// `HotKey::id()` → user-supplied string id (or the accelerator string
/// when no explicit id was provided). Keyed by the OS hotkey id so the
/// event handler can route fast. `LazyLock` (Rust 1.80+) builds the
/// `DashMap` on first deref — direct `ID_MAP.get(...)` calls replace
/// the previous accessor wrapping `OnceLock::get_or_init`.
static ID_MAP: LazyLock<DashMap<u32, String>> = LazyLock::new(DashMap::new);
/// User-supplied string id → registered HotKey. Lets `unregister`
/// recover the HotKey without reparsing.
static HOTKEYS: LazyLock<DashMap<String, HotKey>> = LazyLock::new(DashMap::new);
static HANDLER_INSTALLED: AtomicBool = AtomicBool::new(false);

/// Marker for the `#[tynd::handler]` dispatch table — hotkey state
/// lives in the file-level `MANAGER` / `ID_MAP` / `HOTKEYS` statics.
#[derive(Debug, Default)]
pub struct ShortcutsApi;

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
        HOTKEYS.contains_key(&id)
    }
}

fn manager() -> Result<&'static Mutex<GlobalHotKeyManager>, String> {
    MANAGER
        .get_or_init(|| {
            GlobalHotKeyManager::new().map(Mutex::new).map_err(|e| {
                IpcError::new("shortcuts", "init", format!("GlobalHotKeyManager init failed: {e}"))
                    .into_string()
            })
        })
        .as_ref()
        .map_err(Clone::clone)
}

/// Idempotent installer for the OS-level hotkey event handler. Routes
/// each `Pressed` event into `os::events::emit("shortcut:triggered", ...)`.
fn ensure_handler() {
    if HANDLER_INSTALLED.swap(true, Ordering::AcqRel) {
        return;
    }
    global_hotkey::GlobalHotKeyEvent::set_event_handler(Some(
        |evt: global_hotkey::GlobalHotKeyEvent| {
            if !matches!(evt.state, global_hotkey::HotKeyState::Pressed) {
                return;
            }
            let id = ID_MAP
                .get(&evt.id)
                .map_or_else(|| format!("hotkey-{}", evt.id), |e| e.clone());
            events::emit("shortcut:triggered", &json!({ "id": id }));
        },
    ));
}

fn register_impl(accel: String, id: Option<String>) -> Result<Value, String> {
    let user_id = id.unwrap_or_else(|| accel.clone());

    if HOTKEYS.contains_key(&user_id) {
        return Err(IpcError::new(
            "shortcuts",
            "register",
            format!("id '{user_id}' already registered"),
        )
        .into_string());
    }

    let hotkey = HotKey::from_str(&accel).map_err(|e| {
        IpcError::new("shortcuts", "register", format!("bad accelerator {accel:?}: {e}"))
            .into_string()
    })?;

    ensure_handler();
    manager()?.lock().register(hotkey).map_err(|e| {
        IpcError::new("shortcuts", "register", format!("({accel}): {e}")).into_string()
    })?;

    ID_MAP.insert(hotkey.id(), user_id.clone());
    HOTKEYS.insert(user_id.clone(), hotkey);

    Ok(json!({ "id": user_id }))
}

fn unregister_impl(user_id: &str) -> Result<bool, String> {
    let Some((_, hotkey)) = HOTKEYS.remove(user_id) else {
        return Ok(false);
    };
    ID_MAP.remove(&hotkey.id());
    manager()?.lock().unregister(hotkey).map_err(|e| {
        IpcError::new("shortcuts", "unregister", format!("({user_id}): {e}")).into_string()
    })?;
    Ok(true)
}

fn unregister_all_impl() -> Result<(), String> {
    let ids: Vec<String> = HOTKEYS.iter().map(|e| e.key().clone()).collect();
    let mut err: Option<String> = None;
    for id in &ids {
        if let Err(e) = unregister_impl(id) {
            err.get_or_insert(e);
        }
    }
    err.map_or(Ok(()), Err)
}
