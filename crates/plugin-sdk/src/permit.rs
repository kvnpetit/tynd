use std::{
    fmt,
    num::NonZeroUsize,
    sync::Arc,
    time::{Duration, Instant},
};

use parking_lot::{Condvar, Mutex, MutexGuard};

use crate::{PluginError, PluginResult};

#[derive(Debug)]
struct PoolState {
    /// Permits currently available.
    free: Mutex<usize>,
    /// Signalled whenever a permit comes back.
    returned: Condvar,
}

/// A fixed-capacity pool for bounding concurrent native work.
///
/// Acquired permits return automatically when dropped, including during
/// panic unwinding or early-return error paths.
///
/// Pick the acquisition method by where you run:
///
/// * [`try_acquire`](Self::try_acquire) never blocks; use it from commands
///   when "full" should be an immediate error.
/// * [`acquire_timeout`](Self::acquire_timeout) blocks for a bounded time; use
///   it from commands that may wait briefly.
/// * [`acquire`](Self::acquire) blocks **without a bound**. A command runs on a
///   shared call-pool worker, so parking it here starves unrelated plugin
///   calls (`fs`, `http`, `dialog`, ...). Prefer the two methods above for
///   anything reachable from a command.
#[derive(Clone)]
pub struct BoundedPermitPool {
    state: Arc<PoolState>,
    name: Arc<str>,
    capacity: usize,
}

impl BoundedPermitPool {
    /// Creates a pool containing `capacity` permits.
    ///
    /// # Panics
    ///
    /// Panics when `capacity` is zero because such a pool could never
    /// satisfy an acquisition.
    pub fn new(capacity: usize, name: impl Into<Arc<str>>) -> Self {
        assert!(
            capacity > 0,
            "bounded permit pool capacity must be positive"
        );
        Self {
            state: Arc::new(PoolState {
                free: Mutex::new(capacity),
                returned: Condvar::new(),
            }),
            name: name.into(),
            capacity,
        }
    }

    /// Creates a pool sized to the machine's available parallelism (at least 1).
    pub fn per_core(name: impl Into<Arc<str>>) -> Self {
        Self::new(
            std::thread::available_parallelism().map_or(1, NonZeroUsize::get),
            name,
        )
    }

    /// Park until a permit returns or `deadline` passes; true on timeout.
    /// `None` is an unrepresentable deadline: wait without a bound.
    fn wait_returned(&self, free: &mut MutexGuard<'_, usize>, deadline: Option<Instant>) -> bool {
        if let Some(deadline) = deadline {
            self.state.returned.wait_until(free, deadline).timed_out()
        } else {
            self.state.returned.wait(free);
            false
        }
    }

    fn permit(&self) -> Permit {
        Permit {
            state: Arc::clone(&self.state),
        }
    }

    /// Blocks until one permit is available, with no upper bound.
    ///
    /// Do not call this from a command: it parks a shared call-pool worker.
    /// Use [`try_acquire`](Self::try_acquire) or
    /// [`acquire_timeout`](Self::acquire_timeout) instead.
    pub fn acquire(&self, _operation: &str) -> PluginResult<Permit> {
        let mut free = self.state.free.lock();
        while *free == 0 {
            self.state.returned.wait(&mut free);
        }
        *free -= 1;
        Ok(self.permit())
    }

    /// Takes a permit without blocking. `Ok(None)` when every permit is held.
    pub fn try_acquire(&self, _operation: &str) -> PluginResult<Option<Permit>> {
        let mut free = self.state.free.lock();
        if *free == 0 {
            return Ok(None);
        }
        *free -= 1;
        Ok(Some(self.permit()))
    }

    /// Waits up to `timeout` for a permit. On expiry fails with
    /// `"{operation}: {name} busy (waited {n}s) - retry later"`.
    pub fn acquire_timeout(&self, operation: &str, timeout: Duration) -> PluginResult<Permit> {
        let deadline = Instant::now().checked_add(timeout);
        let mut free = self.state.free.lock();
        while *free == 0 {
            let timed_out = self.wait_returned(&mut free, deadline);
            if timed_out && *free == 0 {
                return Err(PluginError::Custom(format!(
                    "{operation}: {} busy (waited {}s) - retry later",
                    self.name,
                    timeout.as_secs_f32()
                )));
            }
        }
        *free -= 1;
        Ok(self.permit())
    }

    /// Waits up to `timeout` until every permit is back in the pool. Returns
    /// `true` when the pool is idle. Use it to drain in-flight work from
    /// `shutdown()`.
    pub fn wait_idle(&self, timeout: Duration) -> bool {
        let deadline = Instant::now().checked_add(timeout);
        let mut free = self.state.free.lock();
        while *free < self.capacity {
            let timed_out = self.wait_returned(&mut free, deadline);
            if timed_out && *free < self.capacity {
                return false;
            }
        }
        true
    }

    /// Permits available right now.
    pub fn available(&self) -> usize {
        *self.state.free.lock()
    }

    /// Maximum number of simultaneously-held permits.
    pub const fn capacity(&self) -> usize {
        self.capacity
    }
}

impl fmt::Debug for BoundedPermitPool {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BoundedPermitPool")
            .field("name", &self.name)
            .field("capacity", &self.capacity)
            .finish_non_exhaustive()
    }
}

/// An acquired slot from a [`BoundedPermitPool`].
///
/// Dropping this value returns its slot to the originating pool.
pub struct Permit {
    state: Arc<PoolState>,
}

impl fmt::Debug for Permit {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("Permit").finish_non_exhaustive()
    }
}

impl Drop for Permit {
    fn drop(&mut self) {
        *self.state.free.lock() += 1;
        // Wakes both `acquire*` waiters and `wait_idle`.
        self.state.returned.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn try_acquire_is_non_blocking_and_recovers_on_drop() {
        let pool = BoundedPermitPool::new(2, "test pool");
        let a = pool.try_acquire("t").unwrap().unwrap();
        let _b = pool.try_acquire("t").unwrap().unwrap();
        assert!(pool.try_acquire("t").unwrap().is_none());
        assert_eq!(pool.available(), 0);
        drop(a);
        assert_eq!(pool.available(), 1);
        assert!(pool.try_acquire("t").unwrap().is_some());
    }

    #[test]
    fn acquire_timeout_reports_busy_instead_of_blocking_forever() {
        let pool = BoundedPermitPool::new(1, "test pool");
        let held = pool.acquire("t").unwrap();
        let started = Instant::now();
        let error = pool
            .acquire_timeout("t.op", Duration::from_millis(40))
            .unwrap_err()
            .to_string();
        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(error.contains("t.op: test pool busy"), "{error}");
        assert!(error.contains("retry later"), "{error}");
        drop(held);
        assert!(pool
            .acquire_timeout("t.op", Duration::from_millis(40))
            .is_ok());
    }

    #[test]
    fn acquire_timeout_wakes_when_a_permit_returns() {
        let pool = BoundedPermitPool::new(1, "test pool");
        let held = pool.acquire("t").unwrap();
        let releaser = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(30));
            drop(held);
        });
        assert!(pool.acquire_timeout("t", Duration::from_secs(5)).is_ok());
        releaser.join().unwrap();
    }

    #[test]
    fn wait_idle_tracks_outstanding_permits() {
        let pool = BoundedPermitPool::new(2, "test pool");
        assert!(pool.wait_idle(Duration::ZERO));
        let held = pool.acquire("t").unwrap();
        assert!(!pool.wait_idle(Duration::from_millis(20)));
        let releaser = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(30));
            drop(held);
        });
        assert!(pool.wait_idle(Duration::from_secs(5)));
        releaser.join().unwrap();
    }

    #[test]
    fn per_core_has_at_least_one_permit() {
        let pool = BoundedPermitPool::per_core("test pool");
        assert!(pool.capacity() >= 1);
        assert_eq!(pool.available(), pool.capacity());
    }

    #[test]
    fn concurrent_try_acquire_never_exceeds_capacity() {
        let pool = BoundedPermitPool::new(3, "test pool");
        let held = std::sync::Mutex::new(Vec::new());
        std::thread::scope(|scope| {
            for _ in 0..16 {
                scope.spawn(|| {
                    if let Some(permit) = pool.try_acquire("t").unwrap() {
                        held.lock().unwrap().push(permit);
                    }
                });
            }
        });
        assert_eq!(held.lock().unwrap().len(), 3);
    }
}
