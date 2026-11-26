//! Concurrent integer-keyed handle registry for `api::*` modules that hand
//! a `u64` id back to JS and look the underlying resource up later
//! (currently `websocket` sessions).
//!
//! Ids are monotonic `u64`, never reused for the lifetime of the
//! process — IDs are wire-stable from the JS side.
//!
//! Compared to `tynd_plugin_sdk::Registry<T>` (which wraps each value
//! in a `parking_lot::Mutex<T>`), this variant keeps `T` unwrapped —
//! the host modules typically already store `Sender<Cmd>` or
//! `Arc<Something>` that are themselves cheaply cloneable, so the
//! extra mutex would be pure overhead.

use std::sync::atomic::{AtomicU64, Ordering};

use dashmap::DashMap;

/// Sharded handle map with monotonic id allocation. `DashMap::new()`
/// is non-`const`, so consumers wrap the registry in
/// `std::sync::LazyLock` for the static-init use case (still single-
/// alloc on first reach, no `OnceLock::get_or_init` boilerplate).
#[derive(Debug)]
pub(crate) struct Registry<T> {
    map: DashMap<u64, T>,
    next: AtomicU64,
}

impl<T> Registry<T> {
    /// Build an empty registry. IDs start at 1 (0 stays reserved for
    /// "no handle" sentinel use cases on the JS side if any consumer
    /// ever wants one).
    pub(crate) fn new() -> Self {
        Self {
            map: DashMap::new(),
            next: AtomicU64::new(1),
        }
    }

    /// Insert a value and return its freshly minted id. Never blocks —
    /// `DashMap::insert` only locks the target shard, the atomic
    /// fetch-add is wait-free.
    pub(crate) fn insert(&self, value: T) -> u64 {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        self.map.insert(id, value);
        id
    }

    /// Drop the entry with the given id, returning the previously
    /// stored value (so callers can run cleanup on it without an extra
    /// `get` + `remove` dance).
    pub(crate) fn remove(&self, id: u64) -> Option<T> {
        self.map.remove(&id).map(|(_, v)| v)
    }

    /// Snapshot of every live id, in arbitrary order. Used by `list`
    /// dispatch arms.
    pub(crate) fn ids(&self) -> Vec<u64> {
        self.map.iter().map(|e| *e.key()).collect()
    }
}

impl<T: Clone> Registry<T> {
    /// Look up a handle and clone the stored value out. Tailored for
    /// `Sender<Cmd>` / `Arc<…>` payloads where `Clone` is cheap; the
    /// returned value is independent of the registry shard lock so
    /// callers can use it without holding the map.
    pub(crate) fn get_cloned(&self, id: u64) -> Option<T> {
        self.map.get(&id).map(|r| r.value().clone())
    }
}
