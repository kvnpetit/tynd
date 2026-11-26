//! Keep-awake requests (`power.preventSleep` / `power.allowSleep`).
//!
//! Each request holds one platform handle (Windows power request, macOS
//! IOPM assertion, Linux logind inhibitor fd). Dropping the handle releases
//! the request, so the table below only has to track ids and own the
//! handles. Everything is released on host shutdown; after that no new
//! request is accepted.

use std::{
    collections::BTreeMap,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        LazyLock,
    },
};

use parking_lot::Mutex;

use crate::ipc::owner::{self, Owner};

#[cfg(target_os = "windows")]
#[path = "inhibit_windows.rs"]
mod platform;
#[cfg(target_os = "macos")]
#[path = "inhibit_macos.rs"]
mod platform;
#[cfg(all(target_os = "linux", feature = "power-events-linux"))]
#[path = "inhibit_linux.rs"]
mod platform;
#[cfg(not(any(
    target_os = "windows",
    target_os = "macos",
    all(target_os = "linux", feature = "power-events-linux")
)))]
mod platform {
    pub(super) struct Handle;

    pub(super) fn acquire(_display: bool) -> Result<Handle, String> {
        Err("power.preventSleep: not supported on this platform".into())
    }
}

/// Id-keyed owner of live keep-awake handles. Ids are monotonic from 1 and
/// never reused.
pub(super) struct InhibitTable<H> {
    entries: Mutex<BTreeMap<u64, (Option<Owner>, H)>>,
    next: AtomicU64,
    closed: AtomicBool,
}

impl<H> InhibitTable<H> {
    pub(super) fn new() -> Self {
        Self {
            entries: Mutex::new(BTreeMap::new()),
            next: AtomicU64::new(1),
            closed: AtomicBool::new(false),
        }
    }

    /// Create a handle with `create` and register it under a fresh id.
    pub(super) fn acquire_with<F>(&self, create: F) -> Result<u64, String>
    where
        F: FnOnce() -> Result<H, String>,
    {
        if self.closed.load(Ordering::Acquire) {
            return Err(closed_error());
        }
        let handle = create()?;
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let mut entries = self.entries.lock();
        // `release_all` may have run while `create` was in flight.
        if self.closed.load(Ordering::Acquire) {
            drop(entries);
            drop(handle);
            return Err(closed_error());
        }
        // A call still running when its webview committed a new document
        // registers after the release, which already ran: release the request
        // now. Checked under the table lock that `release_owner` also takes
        // (after the generation bump), so a request that passes is seen by it.
        let owner = owner::current();
        if owner.as_ref().is_some_and(|owner| !owner.is_current()) {
            drop(entries);
            drop(handle);
            return Err(gone_error());
        }
        entries.insert(id, (owner, handle));
        Ok(id)
    }

    /// Release one request. Returns whether `id` was live and the caller may
    /// release it; another webview's request answers like an unknown id.
    pub(super) fn release(&self, id: u64) -> bool {
        let handle = {
            let mut entries = self.entries.lock();
            if entries
                .get(&id)
                .is_some_and(|(owner, _)| owner::permits(owner.as_ref()))
            {
                entries.remove(&id)
            } else {
                None
            }
        };
        handle.is_some()
    }

    /// Release the requests `label` made (its JS is gone). Returns how many.
    pub(super) fn release_owner(&self, label: &str) -> usize {
        let released: Vec<_> = {
            let mut entries = self.entries.lock();
            let ids: Vec<u64> = entries
                .iter()
                .filter(|(_, (owner, _))| {
                    owner.as_ref().is_some_and(|owner| owner.label() == label)
                })
                .map(|(id, _)| *id)
                .collect();
            ids.into_iter()
                .filter_map(|id| entries.remove(&id))
                .collect()
        };
        released.len()
    }

    /// Release every request and refuse new ones. Returns how many were live.
    pub(super) fn release_all(&self) -> usize {
        self.closed.store(true, Ordering::Release);
        let entries = std::mem::take(&mut *self.entries.lock());
        entries.len()
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.entries.lock().len()
    }
}

fn closed_error() -> String {
    "power.preventSleep: host is shutting down".into()
}

fn gone_error() -> String {
    "power.preventSleep: the document that requested it is gone".into()
}

static TABLE: LazyLock<InhibitTable<platform::Handle>> = LazyLock::new(InhibitTable::new);

pub(super) fn prevent_sleep(display: bool) -> Result<u64, String> {
    TABLE.acquire_with(|| platform::acquire(display))
}

pub(super) fn allow_sleep(id: u64) -> bool {
    TABLE.release(id)
}

pub(super) fn release_owner(label: &str) {
    TABLE.release_owner(label);
}

pub(super) fn release_all() {
    let released = TABLE.release_all();
    if released > 0 {
        crate::tynd_info!("power: released {released} keep-awake request(s)");
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::ipc::owner::with_caller;

    /// Counts live handles so tests can observe the release.
    struct Probe(Arc<AtomicU64>);

    impl Probe {
        fn new(live: &Arc<AtomicU64>) -> Self {
            live.fetch_add(1, Ordering::SeqCst);
            Self(Arc::clone(live))
        }
    }

    impl Drop for Probe {
        fn drop(&mut self) {
            self.0.fetch_sub(1, Ordering::SeqCst);
        }
    }

    /// Exercises the real OS call: creating and dropping a request is
    /// harmless and needs no privileges.
    #[cfg(target_os = "windows")]
    #[test]
    fn windows_power_request_round_trip() {
        let table = InhibitTable::new();
        let id = table.acquire_with(|| platform::acquire(true)).unwrap();
        assert!(table.release(id));
        let id = table.acquire_with(|| platform::acquire(false)).unwrap();
        assert_eq!(table.release_all(), 1);
        assert!(!table.release(id));
    }

    #[test]
    fn ids_are_monotonic_and_never_reused() {
        let live = Arc::new(AtomicU64::new(0));
        let table = InhibitTable::new();
        let first = table.acquire_with(|| Ok(Probe::new(&live))).unwrap();
        let second = table.acquire_with(|| Ok(Probe::new(&live))).unwrap();
        assert_eq!((first, second), (1, 2));

        assert!(table.release(first));
        let third = table.acquire_with(|| Ok(Probe::new(&live))).unwrap();
        assert_eq!(third, 3);
        assert_eq!(table.len(), 2);
    }

    #[test]
    fn release_drops_the_handle_once() {
        let live = Arc::new(AtomicU64::new(0));
        let table = InhibitTable::new();
        let id = table.acquire_with(|| Ok(Probe::new(&live))).unwrap();
        assert_eq!(live.load(Ordering::SeqCst), 1);

        assert!(table.release(id));
        assert_eq!(live.load(Ordering::SeqCst), 0);
        assert!(!table.release(id), "second release is a no-op");
        assert!(!table.release(999), "unknown id is a no-op");
    }

    #[test]
    fn failed_creation_consumes_no_id() {
        let table: InhibitTable<()> = InhibitTable::new();
        assert!(table.acquire_with(|| Err("denied".into())).is_err());
        assert_eq!(table.acquire_with(|| Ok(())).unwrap(), 1);
    }

    #[test]
    fn another_webviews_request_answers_like_an_unknown_id() {
        let live = Arc::new(AtomicU64::new(0));
        let table = InhibitTable::new();
        let id = with_caller("pw-a", || table.acquire_with(|| Ok(Probe::new(&live)))).unwrap();

        assert!(!with_caller("pw-b", || table.release(id)));
        assert_eq!(live.load(Ordering::SeqCst), 1);
        assert!(with_caller("pw-a", || table.release(id)));
        assert_eq!(live.load(Ordering::SeqCst), 0);

        let id = with_caller("pw-a", || table.acquire_with(|| Ok(Probe::new(&live)))).unwrap();
        assert!(table.release(id), "the backend may release any request");
    }

    #[test]
    fn request_after_the_document_was_replaced_is_released() {
        let live = Arc::new(AtomicU64::new(0));
        let table = InhibitTable::new();
        let label = "pw-late";
        let result = with_caller(label, || {
            // The commit lands while the request is being created.
            owner::bump_generation(label);
            table.acquire_with(|| Ok(Probe::new(&live)))
        });
        assert!(result.is_err());
        assert_eq!(table.len(), 0);
        assert_eq!(live.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn release_owner_drops_only_that_webviews_requests() {
        let live = Arc::new(AtomicU64::new(0));
        let table = InhibitTable::new();
        with_caller("pw-c", || table.acquire_with(|| Ok(Probe::new(&live)))).unwrap();
        with_caller("pw-d", || table.acquire_with(|| Ok(Probe::new(&live)))).unwrap();
        table.acquire_with(|| Ok(Probe::new(&live))).unwrap();

        assert_eq!(table.release_owner("pw-c"), 1);
        assert_eq!(live.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn release_all_drops_everything_and_closes_the_table() {
        let live = Arc::new(AtomicU64::new(0));
        let table = InhibitTable::new();
        table.acquire_with(|| Ok(Probe::new(&live))).unwrap();
        table.acquire_with(|| Ok(Probe::new(&live))).unwrap();

        assert_eq!(table.release_all(), 2);
        assert_eq!(live.load(Ordering::SeqCst), 0);
        assert!(table.acquire_with(|| Ok(Probe::new(&live))).is_err());
        assert_eq!(live.load(Ordering::SeqCst), 0, "no handle is even created");
    }
}
