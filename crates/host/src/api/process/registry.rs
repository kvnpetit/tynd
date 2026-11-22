use std::{
    process::ChildStdin,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, LazyLock,
    },
    time::{Duration, Instant},
};

use dashmap::DashMap;
use parking_lot::{Condvar, Mutex, RwLock, RwLockReadGuard};
use shared_child::SharedChild;

use super::proc_err;
use crate::ipc::owner::Owner;

const MAX_SPAWNED_PROCESSES: usize = 32;

pub(super) struct SpawnEntry {
    pub(super) child: Arc<SharedChild>,
    pub(super) stdin: Mutex<Option<ChildStdin>>,
    /// Identity that spawned the process; its events go to that webview only.
    /// `None` for a backend spawn, or once the spawning document is gone
    /// (default event scope). Read and emitted under one lock, so an event is
    /// never attributed to a document that already released the process.
    owner: Mutex<Option<Owner>>,
    active: Mutex<bool>,
    active_changed: Condvar,
}

impl SpawnEntry {
    /// Current owner label (`None`: backend).
    #[cfg(test)]
    pub(super) fn owner(&self) -> Option<String> {
        self.owner
            .lock()
            .as_ref()
            .map(|owner| owner.label().to_owned())
    }

    /// Lock the owner, first re-owning the process to the backend when the
    /// document that spawned it is already gone. A spawn still running when its
    /// webview committed a new document registers after `release_owner` has
    /// scanned the registry, and would otherwise stay owned by the label.
    fn owner_lock(&self) -> parking_lot::MutexGuard<'_, Option<Owner>> {
        let mut owner = self.owner.lock();
        if owner.as_ref().is_some_and(|owner| !owner.is_current()) {
            *owner = None;
        }
        owner
    }

    /// Re-own the process to the backend if its spawning document is gone. Call
    /// it once the entry is visible in the registry.
    pub(super) fn reown_if_stale(&self) {
        drop(self.owner_lock());
    }

    /// Whether the calling webview may operate on this process.
    pub(super) fn permits_caller(&self) -> bool {
        crate::ipc::owner::permits(self.owner_lock().as_ref())
    }

    /// Emit a per-process event to the current owner.
    pub(super) fn emit(&self, name: &str, data: &serde_json::Value) {
        let owner = self.owner_lock();
        crate::ipc::owner::emit_for(owner.as_ref(), name, data);
    }

    /// Block event-producing workers until the renderer has installed all
    /// per-process listeners and explicitly activates this reservation.
    pub(super) fn wait_until_active(&self) {
        let mut active = self.active.lock();
        while !*active {
            self.active_changed.wait(&mut active);
        }
    }

    /// Like [`wait_until_active`](Self::wait_until_active) but gives up after
    /// `timeout`. Returns whether the reservation was activated in time.
    pub(super) fn wait_until_active_for(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let mut active = self.active.lock();
        while !*active {
            if self
                .active_changed
                .wait_until(&mut active, deadline)
                .timed_out()
            {
                return *active;
            }
        }
        true
    }

    /// Idempotently release every worker belonging to this process.
    pub(super) fn activate(&self) {
        let mut active = self.active.lock();
        if *active {
            return;
        }
        *active = true;
        self.active_changed.notify_all();
    }
}

static REGISTRY: LazyLock<DashMap<u32, Arc<SpawnEntry>>> = LazyLock::new(DashMap::default);
static LIVE_SPAWN_SLOTS: AtomicUsize = AtomicUsize::new(0);
static SHUTTING_DOWN: RwLock<bool> = RwLock::new(false);

pub(super) fn new_entry(
    child: Arc<SharedChild>,
    stdin: Option<ChildStdin>,
    owner: Option<Owner>,
) -> Arc<SpawnEntry> {
    Arc::new(SpawnEntry {
        child,
        stdin: Mutex::new(stdin),
        owner: Mutex::new(owner),
        active: Mutex::new(false),
        active_changed: Condvar::new(),
    })
}

pub(super) fn insert(pid: u32, entry: Arc<SpawnEntry>) {
    REGISTRY.insert(pid, entry);
}

pub(super) fn remove(pid: u32) -> Option<Arc<SpawnEntry>> {
    REGISTRY.remove(&pid).map(|(_, v)| v)
}

pub(super) fn get(pid: u32) -> Option<Arc<SpawnEntry>> {
    REGISTRY.get(&pid).map(|e| e.value().clone())
}

/// Look up `pid` for the calling webview. A process owned by another webview
/// answers like an unknown pid (no existence oracle).
pub(super) fn get_for_caller(pid: u32) -> Option<Arc<SpawnEntry>> {
    get(pid).filter(|entry| entry.permits_caller())
}

/// The document of `label` is gone: re-own its processes to the backend. They
/// keep running (their events now go to every app-local webview, like a
/// backend spawn) and only the backend or an app-local webview can control them.
pub(super) fn release_owner(label: &str) {
    for entry in REGISTRY.iter() {
        let mut owner = entry.value().owner.lock();
        if owner.as_ref().is_some_and(|owner| owner.label() == label) {
            *owner = None;
        }
    }
}

/// Hold this guard until a spawned child and its waiter are fully registered.
///
/// Shutdown takes the write side before snapshotting the registry. This closes
/// the race where cleanup could miss a child between `Command::spawn` and
/// `REGISTRY.insert`.
pub(super) fn admit_spawn() -> Result<RwLockReadGuard<'static, bool>, String> {
    let admission = SHUTTING_DOWN.read();
    if *admission {
        return Err(proc_err("spawn", "host is shutting down; spawn rejected"));
    }
    Ok(admission)
}

pub(super) fn begin_shutdown() -> Vec<(u32, Arc<SpawnEntry>)> {
    let mut shutting_down = SHUTTING_DOWN.write();
    *shutting_down = true;
    REGISTRY
        .iter()
        .map(|entry| (*entry.key(), Arc::clone(entry.value())))
        .collect()
}

pub(super) fn remaining_pids() -> Vec<u32> {
    REGISTRY.iter().map(|entry| *entry.key()).collect()
}

pub(super) fn is_drained() -> bool {
    REGISTRY.is_empty() && LIVE_SPAWN_SLOTS.load(Ordering::Acquire) == 0
}

pub(super) fn reserve_slot() -> Result<(), String> {
    let mut current = LIVE_SPAWN_SLOTS.load(Ordering::Acquire);
    loop {
        if current >= MAX_SPAWNED_PROCESSES {
            return Err(proc_err(
                "spawn",
                format!("too many live child processes (max {MAX_SPAWNED_PROCESSES})"),
            ));
        }
        match LIVE_SPAWN_SLOTS.compare_exchange_weak(
            current,
            current + 1,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => return Ok(()),
            Err(actual) => current = actual,
        }
    }
}

pub(super) fn release_slot() {
    let previous = LIVE_SPAWN_SLOTS.fetch_sub(1, Ordering::AcqRel);
    debug_assert!(previous > 0, "process spawn slot underflow");
}

#[cfg(test)]
mod tests {
    use std::process::{Command, Stdio};

    use super::*;

    fn sleeper_entry() -> Arc<SpawnEntry> {
        let mut command = if cfg!(windows) {
            let mut c = Command::new("ping");
            c.args(["-n", "30", "127.0.0.1"]);
            c
        } else {
            let mut c = Command::new("sleep");
            c.arg("30");
            c
        };
        let child = command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        new_entry(Arc::new(SharedChild::new(child).unwrap()), None, None)
    }

    #[test]
    fn activation_wait_times_out_then_succeeds_after_activate() {
        let entry = sleeper_entry();
        assert!(!entry.wait_until_active_for(Duration::from_millis(30)));
        entry.activate();
        assert!(entry.wait_until_active_for(Duration::from_millis(30)));
        let _ = entry.child.kill();
        let _ = entry.child.wait();
    }

    #[test]
    fn activate_releases_a_blocked_waiter() {
        let entry = sleeper_entry();
        let waiter = {
            let entry = Arc::clone(&entry);
            std::thread::spawn(move || entry.wait_until_active())
        };
        std::thread::sleep(Duration::from_millis(30));
        entry.activate();
        waiter.join().unwrap();
        let _ = entry.child.kill();
        let _ = entry.child.wait();
    }
}
