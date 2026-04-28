//! Session registry shared by the polyfill bridges.
//!
//! `fetch.rs` and `websocket.rs` both maintain a process-global map
//! from JS-side session id (a string minted by the polyfill) to a
//! command-sender that drives the worker thread. Without this helper
//! both modules ended up duplicating:
//!
//! - the `parking_lot::Mutex<HashMap<String, mpsc::Sender<Cmd>>>`
//!   declaration (with the `OnceLock` lazy-init dance),
//! - lookup+forward boilerplate inside every JS-callable function,
//! - cleanup-on-disconnect semantics.
//!
//! `Sessions<C>` lifts the entire pattern out so each polyfill bridge
//! just calls `insert` / `send` / `remove` and the helper handles the
//! locking. We keep a single declaration of `parking_lot::Mutex` per
//! cell — `std::sync::Mutex` would crash every subsequent operation
//! if a holder ever panicked while holding the lock.

use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::mpsc;

/// Concurrent map from JS-side session id → command sender. `C` is the
/// command enum the worker thread consumes (`fetch::Cmd`,
/// `tynd_host::os::websocket::Cmd`, …).
///
/// Methods are intentionally tiny: nothing here owns a worker thread,
/// the polyfill bridge does. This struct is only the routing table.
pub(crate) struct Sessions<C> {
    map: Mutex<HashMap<String, mpsc::Sender<C>>>,
}

impl<C> Sessions<C> {
    /// Build an empty registry. Each bridge module owns one as a
    /// `OnceLock<Sessions<Cmd>>` global so callbacks registered with
    /// `Function::new(ctx.clone(), …)` can reach it without capturing
    /// any state from the surrounding module.
    pub(crate) fn new() -> Self {
        Self {
            map: Mutex::new(HashMap::new()),
        }
    }

    /// Register a session by id. Overwrites any existing entry — the
    /// bridges treat duplicate ids as "the JS side abandoned the old
    /// session and reused the slot".
    pub(crate) fn insert(&self, id: String, tx: mpsc::Sender<C>) {
        self.map.lock().insert(id, tx);
    }

    /// Forward a command to the named session. Silently drops the
    /// command when the session is gone so callers don't have to
    /// worry about the close/abort race window.
    pub(crate) fn send(&self, id: &str, cmd: C) {
        if let Some(tx) = self.map.lock().get(id) {
            let _ = tx.send(cmd);
        }
    }

    /// Drop a session entry. Returns whether anything was removed —
    /// useful when callers want to log a stray cleanup.
    pub(crate) fn remove(&self, id: &str) -> bool {
        self.map.lock().remove(id).is_some()
    }
}

impl<C> Default for Sessions<C> {
    fn default() -> Self {
        Self::new()
    }
}
