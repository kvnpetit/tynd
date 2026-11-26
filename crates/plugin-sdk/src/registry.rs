use std::sync::{
    atomic::{AtomicU64, AtomicUsize, Ordering},
    Arc,
};

use parking_lot::Mutex;

use crate::{PluginError, PluginResult};

/// Concurrent handle table for plugin-side state.
///
/// # Locking
///
/// [`with`](Registry::with) runs its closure while holding the entry's mutex:
/// concurrent `with` calls on the **same id** serialize. The table itself is
/// not locked during the closure (other ids, `insert`, `remove` and `ids` stay
/// free), but a long operation still blocks every other user of that entry.
/// For long work (inference, transcoding, big I/O), store `Arc<M>` and use
/// [`get_arc`](Registry::get_arc) to clone the handle out and run lock-free.
pub struct Registry<T> {
    map: dashmap::DashMap<u64, Arc<Mutex<T>>>,
    next_id: AtomicU64,
    count: AtomicUsize,
    max: usize,
}

impl<T> Registry<T> {
    /// Build an empty registry with the given hard cap.
    pub fn new(max: usize) -> Self {
        Self {
            map: dashmap::DashMap::new(),
            next_id: AtomicU64::new(1),
            count: AtomicUsize::new(0),
            max,
        }
    }

    fn try_reserve(&self) -> PluginResult<()> {
        let prev = self.count.fetch_add(1, Ordering::Relaxed);
        if prev >= self.max {
            self.count.fetch_sub(1, Ordering::Relaxed);
            return Err(PluginError::Custom(format!(
                "registry: max {} handles reached - close some first",
                self.max
            )));
        }
        Ok(())
    }

    /// Insert a value and return its freshly-minted id.
    pub fn insert(&self, value: T) -> PluginResult<u64> {
        self.try_reserve()?;
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        self.map.insert(id, Arc::new(Mutex::new(value)));
        Ok(id)
    }

    /// Allocate a fresh id, build the value with `f(id)`, then insert.
    pub fn insert_with<E>(&self, f: impl FnOnce(u64) -> Result<T, E>) -> PluginResult<u64>
    where
        E: Into<PluginError>,
    {
        self.try_reserve()?;
        // Releases the reservation if `f` fails or unwinds; disarmed once the
        // value is stored, so a panicking constructor cannot leak capacity.
        let reservation = Reservation {
            count: &self.count,
            armed: true,
        };
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let value = f(id).map_err(Into::into)?;
        self.map.insert(id, Arc::new(Mutex::new(value)));
        reservation.keep();
        Ok(id)
    }

    fn entry(&self, id: u64) -> Option<Arc<Mutex<T>>> {
        // The shard read guard is released as soon as the `Arc` is cloned, so
        // a slow `f` never blocks `insert` / `remove` on the same shard.
        self.map.get(&id).map(|entry| Arc::clone(entry.value()))
    }

    /// Run `f` against the value at `id` while holding its per-entry mutex
    /// (and nothing else: the table is not locked). Fails when `id` is absent.
    ///
    /// Do not run long work in `f`; see the type-level docs and
    /// [`get_arc`](Registry::get_arc).
    pub fn with<R>(&self, id: u64, f: impl FnOnce(&mut T) -> PluginResult<R>) -> PluginResult<R> {
        let entry = self
            .entry(id)
            .ok_or_else(|| PluginError::Custom(format!("registry: handle {id} not found")))?;
        let mut guard = entry.lock();
        f(&mut guard)
    }

    /// Like [`with`](Registry::with), but a missing `id` is reported as
    /// `InvalidArg { key, .. }` with the detail `"{operation}: {key} {id} not
    /// found"`. Existence check and access share one lookup, so there is no
    /// `contains()` + `with()` race.
    pub fn with_arg<R>(
        &self,
        id: u64,
        key: &str,
        operation: &str,
        f: impl FnOnce(&mut T) -> PluginResult<R>,
    ) -> PluginResult<R> {
        let entry = self.entry(id).ok_or_else(|| PluginError::InvalidArg {
            key: key.to_string(),
            detail: format!("{operation}: {key} {id} not found"),
        })?;
        let mut guard = entry.lock();
        f(&mut guard)
    }

    /// Run `f` on every live entry (ids first, then one lookup each) and
    /// collect the results. Entries removed in between are skipped; an `f`
    /// error also skips that entry.
    pub fn snapshot<R>(&self, mut f: impl FnMut(u64, &mut T) -> PluginResult<R>) -> Vec<R> {
        self.ids()
            .into_iter()
            .filter_map(|id| self.with(id, |value| f(id, value)).ok())
            .collect()
    }

    /// Drop the entry with the given id. Returns whether anything was removed.
    pub fn remove(&self, id: u64) -> bool {
        if self.map.remove(&id).is_some() {
            self.count.fetch_sub(1, Ordering::Relaxed);
            true
        } else {
            false
        }
    }

    /// Snapshot of every live id, in arbitrary order.
    pub fn ids(&self) -> Vec<u64> {
        self.map.iter().map(|e| *e.key()).collect()
    }

    /// O(1) membership check.
    pub fn contains(&self, id: u64) -> bool {
        self.map.contains_key(&id)
    }

    /// Number of live entries.
    pub fn len(&self) -> usize {
        self.count.load(Ordering::Relaxed)
    }

    /// True when no entries are live.
    pub fn is_empty(&self) -> bool {
        self.count.load(Ordering::Relaxed) == 0
    }
}

impl<M> Registry<Arc<M>> {
    /// Clone the shared value out of the registry. The entry lock is held only
    /// for the clone, so a long operation on the returned `Arc` never blocks
    /// `ids`, `remove`, `snapshot` or another run on the same handle.
    pub fn get_arc(&self, id: u64) -> PluginResult<Arc<M>> {
        self.with(id, |value| Ok(Arc::clone(value)))
    }

    /// [`get_arc`](Registry::get_arc) with a missing `id` reported as
    /// `InvalidArg { key, .. }` (see [`with_arg`](Registry::with_arg)).
    pub fn get_arc_arg(&self, id: u64, key: &str, operation: &str) -> PluginResult<Arc<M>> {
        self.with_arg(id, key, operation, |value| Ok(Arc::clone(value)))
    }
}

impl<T> std::fmt::Debug for Registry<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Registry")
            .field("len", &self.len())
            .field("max", &self.max)
            .finish_non_exhaustive()
    }
}

/// Capacity reservation taken by `try_reserve`; releases it on drop unless kept.
struct Reservation<'a> {
    count: &'a AtomicUsize,
    armed: bool,
}

impl Reservation<'_> {
    fn keep(mut self) {
        self.armed = false;
    }
}

impl Drop for Reservation<'_> {
    fn drop(&mut self) {
        if self.armed {
            self.count.fetch_sub(1, Ordering::Relaxed);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{sync::mpsc, time::Duration};

    use super::*;

    struct FakeModel {
        calls: AtomicUsize,
    }

    #[test]
    fn panicking_constructor_releases_its_reservation() {
        let registry: Registry<u32> = Registry::new(1);
        for _ in 0..3 {
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let _ = registry.insert_with(|_| -> Result<u32, PluginError> { panic!("boom") });
            }));
            assert!(outcome.is_err());
        }
        assert!(registry.insert_with(|_| Ok::<_, PluginError>(1)).is_ok());
        assert!(registry.insert(2).is_err(), "cap of 1 still enforced");
    }
    #[test]
    fn with_arg_missing_id_is_invalid_arg() {
        let registry: Registry<u32> = Registry::new(4);
        match registry.with_arg(99, "modelId", "test.op", |_| Ok(())) {
            Err(PluginError::InvalidArg { key, detail }) => {
                assert_eq!(key, "modelId");
                assert!(detail.contains("test.op"), "{detail}");
                assert!(detail.contains("99 not found"), "{detail}");
            },
            other => panic!("expected InvalidArg, got {other:?}"),
        }
        let id = registry.insert(5).unwrap();
        assert_eq!(registry.with_arg(id, "k", "op", |v| Ok(*v)).unwrap(), 5);
    }

    #[test]
    fn get_arc_arg_missing_id_is_invalid_arg() {
        let registry: Registry<Arc<u32>> = Registry::new(4);
        assert!(matches!(
            registry.get_arc_arg(1, "modelId", "op"),
            Err(PluginError::InvalidArg { .. })
        ));
        assert!(registry.get_arc(1).is_err());
    }

    #[test]
    fn snapshot_visits_every_live_entry() {
        let registry: Registry<u32> = Registry::new(8);
        let a = registry.insert(1).unwrap();
        let b = registry.insert(2).unwrap();
        let c = registry.insert(3).unwrap();
        registry.remove(b);
        let mut seen = registry.snapshot(|id, value| Ok((id, *value)));
        seen.sort_unstable();
        assert_eq!(seen, vec![(a, 1), (c, 3)]);
    }

    #[test]
    fn with_does_not_lock_the_table() {
        let registry: Arc<Registry<u32>> = Arc::new(Registry::new(8));
        let id = registry.insert(1).unwrap();
        let (inside_tx, inside_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel::<()>();
        let worker_registry = Arc::clone(&registry);
        let worker = std::thread::spawn(move || {
            worker_registry
                .with(id, |_| {
                    inside_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                    Ok(())
                })
                .unwrap();
        });
        inside_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        // Another id and the table itself stay usable while `id` is busy.
        let (done_tx, done_rx) = mpsc::channel();
        let probe_registry = Arc::clone(&registry);
        let probe = std::thread::spawn(move || {
            let other = probe_registry.insert(2).unwrap();
            let ids = probe_registry.ids().len();
            probe_registry.remove(other);
            done_tx.send(ids).unwrap();
        });
        assert_eq!(
            done_rx
                .recv_timeout(Duration::from_secs(5))
                .expect("table operations blocked behind a busy entry"),
            2
        );
        release_tx.send(()).unwrap();
        worker.join().unwrap();
        probe.join().unwrap();
    }

    #[test]
    fn long_operation_on_an_arc_does_not_hold_the_registry_lock() {
        let models: Arc<Registry<Arc<FakeModel>>> = Arc::new(Registry::new(4));
        let id = models
            .insert(Arc::new(FakeModel {
                calls: AtomicUsize::new(0),
            }))
            .unwrap();
        let (inside_tx, inside_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel::<()>();

        let worker_models = Arc::clone(&models);
        let worker = std::thread::spawn(move || {
            let model = worker_models.get_arc(id).unwrap();
            // Simulated inference: running, blocked, with the Arc cloned out.
            model.calls.fetch_add(1, Ordering::SeqCst);
            inside_tx.send(()).unwrap();
            release_rx.recv().unwrap();
        });
        inside_rx.recv_timeout(Duration::from_secs(5)).unwrap();

        // While the "inference" is blocked, registry operations must not block,
        // not even on the same id.
        let (done_tx, done_rx) = mpsc::channel();
        let probe_models = Arc::clone(&models);
        let probe = std::thread::spawn(move || {
            let second = probe_models.get_arc(id).unwrap();
            let ids = probe_models.ids();
            let second_calls = second.calls.load(Ordering::SeqCst);
            let removed = probe_models.remove(id);
            done_tx.send((ids, second_calls, removed)).unwrap();
        });
        let (ids, second_calls, removed) = done_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("registry operations blocked while a model was running");
        assert_eq!(ids, vec![id]);
        assert_eq!(second_calls, 1);
        assert!(removed);

        release_tx.send(()).unwrap();
        worker.join().unwrap();
        probe.join().unwrap();
    }
}
