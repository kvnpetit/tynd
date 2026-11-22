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
//! plus event-driven update) we keep a single one: install the listener from
//! the TS client at app startup and let `keys:set` events flow.
//!
//! ## Combo grammar
//!
//! `Ctrl+Shift+Alt+Meta+<key>` where `<key>` is the JS `event.key` value
//! (case-sensitive). Modifier order is normalised on insert so callers can
//! write `"Shift+Ctrl+R"` and `"Ctrl+Shift+R"` interchangeably.

use std::{collections::BTreeSet, sync::LazyLock};

use parking_lot::RwLock;
use serde_json::{json, Value};

use crate::ipc::error::IpcError;

use crate::api::HostApi;
use crate::ipc::events;
use crate::tynd;

/// Cap on registered combos. Each entry is a short heap string in a
/// `BTreeSet`; 256 covers any realistic UX (typical apps register
/// 5–10 combos) while bounding the per-keydown lookup set.
const MAX_COMBOS: usize = 256;

static DISABLED: LazyLock<RwLock<BTreeSet<String>>> =
    LazyLock::new(|| RwLock::new(BTreeSet::new()));

/// Dispatch marker — disabled-combo state lives in `DISABLED`.
#[derive(Debug, Default)]
pub(crate) struct KeysApi;

/// Row of the host API table (`api::HOST_APIS`).
pub(crate) const API: HostApi = HostApi::new("keys", |method, args| dispatch(method, &args));

#[tynd::handler(api = "keys")]
impl KeysApi {
    /// Mark `combos` as disabled. Modifier order is normalised
    /// (`Shift+Ctrl+R` and `Ctrl+Shift+R` collapse to one entry).
    /// Errors when the cap of 256 entries is reached.
    pub fn disable(combos: Vec<String>) -> Result<(), String> {
        let snapshot = {
            let mut state = DISABLED.write();
            for combo in combos.into_iter().map(normalise_combo) {
                if state.len() >= MAX_COMBOS && !state.contains(&combo) {
                    return Err(IpcError::new(
                        "keys",
                        "disable",
                        format!("max {MAX_COMBOS} combos reached — call clear() or enable() first"),
                    )
                    .into_string());
                }
                state.insert(combo);
            }
            snapshot_combos(&state)
        };
        broadcast(snapshot);
        Ok(())
    }

    /// Re-enable `combos` (remove them from the disabled set).
    pub fn enable(combos: Vec<String>) {
        let snapshot = {
            let mut state = DISABLED.write();
            for combo in combos {
                state.remove(&normalise_combo(combo));
            }
            snapshot_combos(&state)
        };
        broadcast(snapshot);
    }

    /// List every currently-disabled combo, in canonical
    /// (BTreeSet-sorted) order.
    pub fn list() -> Value {
        Value::Array(snapshot_combos(&DISABLED.read()))
    }

    /// Drop every disabled combo and broadcast `keys:set` so the
    /// frontend rebuilds its mirror.
    pub fn clear() {
        let snapshot = {
            let mut state = DISABLED.write();
            state.clear();
            snapshot_combos(&state)
        };
        broadcast(snapshot);
    }
}

fn snapshot_combos(state: &BTreeSet<String>) -> Vec<Value> {
    state.iter().map(|c| Value::String(c.clone())).collect()
}

fn broadcast(combos: Vec<Value>) {
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
