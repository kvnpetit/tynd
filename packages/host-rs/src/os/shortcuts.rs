//! System-wide keyboard shortcuts via `global-hotkey`.
//!
//! Shortcuts fire even when the app doesn't have focus — Windows uses
//! `RegisterHotKey`, macOS registers an Event Tap, Linux uses XGrabKey /
//! portal. The user-supplied string id is echoed back in the
//! `shortcut:triggered { id }` event so apps can dispatch multiple
//! shortcuts through one listener.

use global_hotkey::{hotkey::HotKey, GlobalHotKeyManager};
use parking_lot::Mutex;
use serde_json::{json, Value};
use std::str::FromStr as _;
use std::sync::OnceLock;

use super::events;

static MANAGER: OnceLock<Mutex<GlobalHotKeyManager>> = OnceLock::new();
/// Maps `HotKey::id()` (u32 the OS actually fires) to the user's string id.
static ID_MAP: OnceLock<dashmap::DashMap<u32, String>> = OnceLock::new();
/// Maps the user's string id to the registered `HotKey` so `unregister`
/// can find it again without reparsing.
static HOTKEYS: OnceLock<dashmap::DashMap<String, HotKey>> = OnceLock::new();

pub fn dispatch(method: &str, args: &Value) -> Result<Value, String> {
    match method {
        "register" => register(args),
        "unregister" => unregister(args),
        "unregisterAll" => unregister_all(),
        "isRegistered" => Ok(Value::Bool(is_registered(args))),
        _ => Err(format!("shortcuts.{method}: unknown method")),
    }
}

fn manager() -> Result<&'static Mutex<GlobalHotKeyManager>, String> {
    if let Some(m) = MANAGER.get() {
        return Ok(m);
    }
    let mgr = GlobalHotKeyManager::new()
        .map_err(|e| format!("shortcuts: GlobalHotKeyManager init failed: {e}"))?;
    let _ = MANAGER.set(Mutex::new(mgr));
    Ok(MANAGER.get().expect("MANAGER just set"))
}

fn id_map() -> &'static dashmap::DashMap<u32, String> {
    ID_MAP.get_or_init(dashmap::DashMap::default)
}

fn hotkeys() -> &'static dashmap::DashMap<String, HotKey> {
    HOTKEYS.get_or_init(dashmap::DashMap::default)
}

/// Subscribe to `GlobalHotKeyEvent`. Called once from `app.rs` at startup —
/// parallel to how `muda::MenuEvent` + `tray_icon::TrayIconEvent` are wired.
pub fn install_event_handler() {
    global_hotkey::GlobalHotKeyEvent::set_event_handler(Some(
        move |evt: global_hotkey::GlobalHotKeyEvent| {
            if !matches!(evt.state, global_hotkey::HotKeyState::Pressed) {
                return;
            }
            let id = id_map()
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
        .ok_or_else(|| "shortcuts.register: missing 'accelerator'".to_string())?;
    let user_id = args
        .get("id")
        .and_then(Value::as_str)
        .map_or_else(|| accel.to_string(), String::from);

    if hotkeys().contains_key(&user_id) {
        return Err(format!(
            "shortcuts.register: id '{user_id}' already registered"
        ));
    }

    let hotkey = HotKey::from_str(accel)
        .map_err(|e| format!("shortcuts.register: bad accelerator {accel:?}: {e}"))?;

    manager()?
        .lock()
        .register(hotkey)
        .map_err(|e| format!("shortcuts.register({accel}): {e}"))?;

    id_map().insert(hotkey.id(), user_id.clone());
    hotkeys().insert(user_id.clone(), hotkey);

    Ok(json!({ "id": user_id }))
}

fn unregister(args: &Value) -> Result<Value, String> {
    let user_id = args
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| "shortcuts.unregister: missing 'id'".to_string())?;
    let Some((_, hotkey)) = hotkeys().remove(user_id) else {
        return Ok(Value::Bool(false));
    };
    id_map().remove(&hotkey.id());
    manager()?
        .lock()
        .unregister(hotkey)
        .map_err(|e| format!("shortcuts.unregister({user_id}): {e}"))?;
    Ok(Value::Bool(true))
}

fn unregister_all() -> Result<Value, String> {
    let ids: Vec<String> = hotkeys().iter().map(|e| e.key().clone()).collect();
    let mut err: Option<String> = None;
    for id in &ids {
        if let Err(e) = unregister(&json!({ "id": id })) {
            err.get_or_insert(e);
        }
    }
    if let Some(e) = err {
        Err(e)
    } else {
        Ok(Value::Null)
    }
}

fn is_registered(args: &Value) -> bool {
    args.get("id")
        .and_then(Value::as_str)
        .is_some_and(|id| hotkeys().contains_key(id))
}
