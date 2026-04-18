//! QuickJS backend lifecycle: spawn the worker thread, relay
//! `BackendCall`s to it, wire up dev-mode hot reload.
//!
//! Internal layout:
//! - `thread.rs` — the worker thread body (bridges, eval, message loop)
//! - `timer.rs`  — the single-thread setTimeout/setInterval scheduler
//! - `globals.js` — raw JS installed before the user bundle
//!
//! The `JsMsg` enum is `pub(crate)` so the polyfill layer can post
//! `PolyfillEvent` messages from background threads (see
//! `polyfills::fetch` / `polyfills::websocket`).

use serde_json::Value;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use tynd_host::{
    runtime::{BackendBridge, BackendCall, BackendConfig, BackendEvent},
    tynd_log,
};

mod thread;
mod timer;

/// Messages consumed by the JS worker thread. Timer/polyfill variants are
/// produced by background threads and drained between user calls so the
/// QuickJS context stays on a single thread.
pub(crate) enum JsMsg {
    Call(BackendCall),
    TimerFire(u32),
    /// Event to deliver to an in-backend JS polyfill (fetch, WebSocket, ...).
    /// Distinct from `BackendEvent::Emit` which is routed to the frontend.
    PolyfillEvent {
        name: String,
        data: Value,
    },
}

/// Dev-mode handle to hot-reload the QuickJS backend without tearing down
/// the host process or the WebView. Mirrors full-mode's `ReloadHandle`.
#[derive(Clone)]
pub(crate) struct ReloadHandle {
    bundle_path: String,
    /// The forwarder thread writes to this slot. On reload we swap its
    /// contents so the next message goes to the new JS thread.
    js_tx_slot: Arc<Mutex<mpsc::Sender<JsMsg>>>,
    event_tx: mpsc::Sender<BackendEvent>,
}

impl ReloadHandle {
    pub(crate) fn reload(&self) {
        let bundle_code = match std::fs::read_to_string(&self.bundle_path) {
            Ok(s) => s,
            Err(e) => {
                let _ = self.event_tx.send(BackendEvent::Error {
                    message: format!("Cannot read bundle '{}': {e}", self.bundle_path),
                });
                return;
            },
        };

        // New JS thread with its own channel pair. Dropping the old sender
        // (via slot swap) closes the old thread's recv loop cleanly.
        let (new_js_tx, new_js_rx) = mpsc::channel::<JsMsg>();
        let (cfg_tx, cfg_rx) = mpsc::sync_channel::<BackendConfig>(1);
        let js_tx_for_thread = new_js_tx.clone();
        let event_tx = self.event_tx.clone();

        std::thread::spawn(move || {
            thread::js_thread_main(bundle_code, js_tx_for_thread, new_js_rx, cfg_tx, event_tx);
        });

        match cfg_rx.recv() {
            Ok(_) => {
                // Swap last so no message races to the old thread.
                *self.js_tx_slot.lock().unwrap() = new_js_tx;
                let _ = self.event_tx.send(BackendEvent::Reload);
            },
            Err(_) => {
                let _ = self.event_tx.send(BackendEvent::Error {
                    message: "Backend reload failed: bundle did not call app.start()".into(),
                });
            },
        }
    }
}

/// Start the QuickJS backend.
///
/// - Loads the bundle at `bundle_path` in an embedded QuickJS runtime
/// - Blocks until the bundle sets `globalThis.__tynd_config__`
/// - Returns a `BackendBridge` plus a `ReloadHandle` for dev-mode hot reload
pub(crate) fn start(
    bundle_path: &str,
    frontend_dir: Option<String>,
    dev_url: Option<String>,
    icon_path: Option<String>,
) -> (BackendBridge, ReloadHandle) {
    let bundle_code = std::fs::read_to_string(bundle_path).unwrap_or_else(|e| {
        tynd_log!("Cannot read bundle '{bundle_path}': {e}");
        tynd_log!("Build first: tynd build");
        std::process::exit(1);
    });

    let (js_tx, js_rx) = mpsc::channel::<JsMsg>();
    let (cfg_tx, cfg_rx) = mpsc::sync_channel::<BackendConfig>(1);
    let (event_tx, event_rx) = mpsc::channel::<BackendEvent>();
    let (call_tx, call_rx) = mpsc::channel::<BackendCall>();

    // JS sender behind a mutex so ReloadHandle can swap it atomically.
    let js_tx_slot: Arc<Mutex<mpsc::Sender<JsMsg>>> = Arc::new(Mutex::new(js_tx.clone()));

    // Forwarder: BackendCall -> current JS thread (via the slot).
    {
        let slot = js_tx_slot.clone();
        std::thread::spawn(move || {
            while let Ok(call) = call_rx.recv() {
                let tx = slot.lock().unwrap().clone();
                let _ = tx.send(JsMsg::Call(call));
            }
        });
    }

    let js_tx_for_thread = js_tx.clone();
    let event_tx_for_thread = event_tx.clone();
    std::thread::spawn(move || {
        thread::js_thread_main(
            bundle_code,
            js_tx_for_thread,
            js_rx,
            cfg_tx,
            event_tx_for_thread,
        );
    });

    let mut config = cfg_rx.recv().unwrap_or_else(|_| {
        tynd_log!("JS thread died before sending config — check your backend calls app.start()");
        std::process::exit(1);
    });

    if frontend_dir.is_some() {
        config.frontend_dir = frontend_dir;
    }
    if dev_url.is_some() {
        config.dev_url = dev_url;
    }
    if icon_path.is_some() {
        config.icon_path = icon_path;
    }

    let bridge = BackendBridge {
        config,
        call_tx,
        event_rx,
    };
    let reload = ReloadHandle {
        bundle_path: bundle_path.to_string(),
        js_tx_slot,
        event_tx,
    };
    (bridge, reload)
}
