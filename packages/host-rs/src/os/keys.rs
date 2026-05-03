//! Browser key intercept — disables default webview shortcuts (Ctrl+R, F5,
//! Ctrl+P, …) per app config.
//!
//! ## Wiring model
//!
//! The host owns the canonical set of disabled combos so a webview reload
//! doesn't lose state. Every mutation re-broadcasts `keys:set` carrying
//! the full list; the TS client (`keys.install()`) keeps a `Set` mirror in
//! `window.__tynd_keys_disabled__` and a `keydown` capture listener
//! consults it to call `event.preventDefault()` for matching combos.
//!
//! ## Why no preload-script injection of the live list
//!
//! `wry`'s `with_initialization_script` runs once per webview before the
//! page loads — it can ship the *initial* list, but later mutations would
//! still need a runtime channel. Rather than maintain two paths (preload
//! + event-driven update) we keep a single one: install the listener from
//! the TS client at app startup and let `keys:set` events flow.
//!
//! ## Combo grammar
//!
//! `Ctrl+Shift+Alt+Meta+<key>` where `<key>` is the JS `event.key` value
//! (case-sensitive). Modifier order is normalised on insert so callers can
//! write `"Shift+Ctrl+R"` and `"Ctrl+Shift+R"` interchangeably.

use std::{
    collections::BTreeSet,
    sync::LazyLock,
};

use parking_lot::RwLock;
use serde_json::{json, Value};

use super::events;
use crate::ipc_error::IpcError;

/// Cap on registered combos. Each entry is a short heap string in a
/// `BTreeSet`; 256 covers any realistic UX (typical apps register
/// 5–10 combos) while bounding the per-keydown lookup set.
const MAX_COMBOS: usize = 256;

/// `LazyLock` initializes the inner `RwLock<BTreeSet<String>>` on
/// first access — replaces a `slot()` accessor wrapping
/// `OnceLock::get_or_init`.
static DISABLED: LazyLock<RwLock<BTreeSet<String>>> = LazyLock::new(|| RwLock::new(BTreeSet::new()));

pub fn dispatch(method: &str, args: &Value) -> Result<Value, String> {
    match method {
        "disable" => disable(args),
        "enable" => enable(args),
        "list" => Ok(list()),
        "clear" => clear(),
        _ => Err(IpcError::unknown_method("keys", method).into_string()),
    }
}

/// Snapshot of currently-disabled combos. Sorted (BTreeSet order) so the
/// frontend `Set` rebuild is deterministic — useful for tests and stable
/// devtools output.
pub fn current_disabled() -> Vec<String> {
    DISABLED.read().iter().cloned().collect()
}

fn disable(args: &Value) -> Result<Value, String> {
    let combos = combos_arg(args)?;
    {
        let mut state = DISABLED.write();
        for combo in combos.into_iter().map(normalise_combo) {
            if state.len() >= MAX_COMBOS && !state.contains(&combo) {
                return Err(IpcError::new(
                    "keys",
                    "disable",
                    format!(
                        "max {MAX_COMBOS} combos reached — call clear() or enable() first"
                    ),
                )
                .into_string());
            }
            state.insert(combo);
        }
    }
    broadcast();
    Ok(Value::Null)
}

fn enable(args: &Value) -> Result<Value, String> {
    let combos = combos_arg(args)?;
    {
        let mut state = DISABLED.write();
        for combo in combos {
            state.remove(&normalise_combo(combo));
        }
    }
    broadcast();
    Ok(Value::Null)
}

fn list() -> Value {
    Value::Array(
        DISABLED
            .read()
            .iter()
            .map(|c| Value::String(c.clone()))
            .collect(),
    )
}

fn clear() -> Result<Value, String> {
    DISABLED.write().clear();
    broadcast();
    Ok(Value::Null)
}

fn combos_arg(args: &Value) -> Result<Vec<String>, String> {
    let arr = args
        .get("combos")
        .and_then(Value::as_array)
        .ok_or_else(|| String::from(IpcError::missing_arg("keys", "*", "combos")))?;
    let mut out = Vec::with_capacity(arr.len());
    for v in arr {
        let s = v
            .as_str()
            .ok_or_else(|| {
                IpcError::new("keys", "*", "'combos' must contain strings").into_string()
            })?;
        out.push(s.to_string());
    }
    Ok(out)
}

fn broadcast() {
    let combos: Vec<Value> = DISABLED
        .read()
        .iter()
        .map(|c| Value::String(c.clone()))
        .collect();
    events::emit("keys:set", &json!({ "combos": combos }));
}

/// Normalise modifier ordering so `"Shift+Ctrl+R"` and `"Ctrl+Shift+R"`
/// don't dedupe as separate entries. Final shape:
/// `Ctrl+Shift+Alt+Meta+<key>` with the trailing key kept verbatim
/// (case-sensitive — `"k"` and `"K"` are distinct because shift is
/// already part of the modifier state).
fn normalise_combo(raw: String) -> String {
    let mut parts: Vec<String> = raw
        .split('+')
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
        .collect();
    if parts.is_empty() {
        return raw;
    }

    let key = parts.pop().unwrap_or_default();

    let mut ctrl = false;
    let mut shift = false;
    let mut alt = false;
    let mut meta = false;
    for p in &parts {
        match p.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => ctrl = true,
            "shift" => shift = true,
            "alt" | "option" => alt = true,
            "meta" | "cmd" | "command" | "super" | "win" => meta = true,
            // Unknown modifier: keep the input verbatim so callers can
            // see what they typed in `list()` output.
            _ => {
                let mut out = parts.join("+");
                out.push('+');
                out.push_str(&key);
                return out;
            },
        }
    }

    let mut out = String::with_capacity(raw.len());
    if ctrl {
        out.push_str("Ctrl+");
    }
    if shift {
        out.push_str("Shift+");
    }
    if alt {
        out.push_str("Alt+");
    }
    if meta {
        out.push_str("Meta+");
    }
    out.push_str(&key);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use parking_lot::Mutex;

    // Tests share the process-global `DISABLED` slot. Run them serially
    // through a single guard so concurrent `cargo test` workers don't
    // observe each other's mutations.
    static SERIAL: Mutex<()> = Mutex::new(());

    fn reset() {
        DISABLED.write().clear();
    }

    #[test]
    fn disable_then_list_round_trip() {
        let _g = SERIAL.lock();
        reset();
        dispatch("disable", &json!({ "combos": ["Ctrl+R"] })).unwrap();
        let out = dispatch("list", &json!(null)).unwrap();
        assert_eq!(out, json!(["Ctrl+R"]));
    }

    #[test]
    fn enable_drops_combo() {
        let _g = SERIAL.lock();
        reset();
        dispatch("disable", &json!({ "combos": ["Ctrl+R", "F5", "Ctrl+P"] })).unwrap();
        dispatch("enable", &json!({ "combos": ["Ctrl+R"] })).unwrap();
        let out = dispatch("list", &json!(null)).unwrap();
        let arr = out.as_array().unwrap();
        assert_eq!(arr.len(), 2);
        assert!(!arr.iter().any(|v| v == &json!("Ctrl+R")));
    }

    #[test]
    fn clear_empties_list() {
        let _g = SERIAL.lock();
        reset();
        dispatch("disable", &json!({ "combos": ["Ctrl+R", "F5"] })).unwrap();
        dispatch("clear", &json!(null)).unwrap();
        let out = dispatch("list", &json!(null)).unwrap();
        assert_eq!(out, json!([]));
    }

    #[test]
    fn modifier_ordering_is_canonical() {
        let _g = SERIAL.lock();
        reset();
        dispatch("disable", &json!({ "combos": ["Shift+Ctrl+I"] })).unwrap();
        let out = dispatch("list", &json!(null)).unwrap();
        assert_eq!(out, json!(["Ctrl+Shift+I"]));
    }

    #[test]
    fn duplicate_disable_is_idempotent() {
        let _g = SERIAL.lock();
        reset();
        dispatch("disable", &json!({ "combos": ["Ctrl+R"] })).unwrap();
        dispatch("disable", &json!({ "combos": ["Ctrl+R"] })).unwrap();
        let out = dispatch("list", &json!(null)).unwrap();
        assert_eq!(out, json!(["Ctrl+R"]));
    }

    #[test]
    fn unknown_method_errors() {
        let _g = SERIAL.lock();
        reset();
        let err = dispatch("wat", &json!(null)).unwrap_err();
        assert!(err.contains("unknown method"));
    }

    #[test]
    fn missing_combos_errors() {
        let _g = SERIAL.lock();
        reset();
        let err = dispatch("disable", &json!({})).unwrap_err();
        assert!(err.contains("missing 'combos'"));
    }
}
