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
use crate::ipc_error::IpcError;

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

pub fn dispatch(method: &str, args: &Value) -> Result<Value, String> {
    match method {
        "register" => register(args),
        "unregister" => unregister(args),
        "unregisterAll" => unregister_all(),
        "isRegistered" => Ok(Value::Bool(is_registered(args))),
        _ => Err(IpcError::unknown_method("shortcuts", method).into_string()),
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

fn register(args: &Value) -> Result<Value, String> {
    let accel = args
        .get("accelerator")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            String::from(IpcError::missing_arg("shortcuts", "register", "accelerator"))
        })?
        .to_string();
    let user_id = args
        .get("id")
        .and_then(Value::as_str)
        .map_or_else(|| accel.clone(), str::to_string);

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

fn unregister(args: &Value) -> Result<Value, String> {
    // The TS client passes the bare id string (not an object) on this path.
    let user_id = args
        .as_str()
        .map(str::to_string)
        .or_else(|| args.get("id").and_then(Value::as_str).map(str::to_string))
        .ok_or_else(|| String::from(IpcError::missing_arg("shortcuts", "unregister", "id")))?;

    let Some((_, hotkey)) = HOTKEYS.remove(&user_id) else {
        return Ok(Value::Bool(false));
    };
    ID_MAP.remove(&hotkey.id());
    manager()?.lock().unregister(hotkey).map_err(|e| {
        IpcError::new("shortcuts", "unregister", format!("({user_id}): {e}")).into_string()
    })?;
    Ok(Value::Bool(true))
}

fn unregister_all() -> Result<Value, String> {
    let ids: Vec<String> = HOTKEYS.iter().map(|e| e.key().clone()).collect();
    let mut err: Option<String> = None;
    for id in &ids {
        if let Err(e) = unregister(&Value::String(id.clone())) {
            err.get_or_insert(e);
        }
    }
    err.map_or(Ok(Value::Null), Err)
}

fn is_registered(args: &Value) -> bool {
    args.as_str()
        .map(str::to_string)
        .or_else(|| args.get("id").and_then(Value::as_str).map(str::to_string))
        .is_some_and(|id| HOTKEYS.contains_key(&id))
}
