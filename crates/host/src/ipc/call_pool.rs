//! Bounded worker pool for OS-API dispatch.
//!
//! Caps steady-state threads at `available_parallelism * 4` (clamped to
//! [8, 64]). Dialogs block on user input, so we size for blocked workers,
//! not CPU-bound throughput.
//!
//! Workers are **lazy**: threads spawn on demand up to `max_workers` instead
//! of eagerly at startup. An app that never calls an OS API pays zero thread
//! overhead. Idle workers stay parked on the condvar — they're cheap
//! (stacks are reserved virtually, the kernel only commits pages on use).
//!
//! Backpressure policy: the queue is bounded and saturated submissions fail
//! immediately. Renderer IPC can then reject the Promise instead of blocking
//! the UI thread behind long-running native calls.

use std::{
    collections::VecDeque,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, LazyLock,
    },
    thread,
    thread::JoinHandle,
    time::{Duration, Instant},
};

use parking_lot::{Condvar, Mutex};

type Job = Box<dyn FnOnce() + Send + 'static>;

#[derive(Debug)]
pub(crate) enum SubmitError {
    Saturated { capacity: usize },
    ThreadSpawn(std::io::Error),
    ShuttingDown,
}

impl std::fmt::Display for SubmitError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Saturated { capacity } => {
                write!(formatter, "OS call queue is saturated ({capacity} pending)")
            },
            Self::ThreadSpawn(error) => write!(formatter, "OS worker thread spawn failed: {error}"),
            Self::ShuttingDown => formatter.write_str("OS call pool is shutting down"),
        }
    }
}

#[derive(Default)]
struct PoolState {
    queue: VecDeque<Job>,
    joins: Vec<JoinHandle<()>>,
    shutting_down: bool,
}

struct Pool {
    state: Mutex<PoolState>,
    not_empty: Condvar,
    max_workers: usize,
    max_queue: usize,
    alive_workers: AtomicUsize,
}

impl Pool {
    fn new(max_workers: usize, max_queue: usize) -> Self {
        debug_assert!(max_workers > 0);
        debug_assert!(max_queue > 0);
        Self {
            state: Mutex::new(PoolState {
                queue: VecDeque::with_capacity(max_queue.min(32)),
                joins: Vec::with_capacity(max_workers),
                shutting_down: false,
            }),
            not_empty: Condvar::new(),
            max_workers,
            max_queue,
            alive_workers: AtomicUsize::new(0),
        }
    }
}

/// Process-wide pool. `LazyLock` builds the inner `Pool` on first
/// deref so a host that never calls an OS API pays zero cost.
static POOL: LazyLock<Arc<Pool>> = LazyLock::new(|| {
    Arc::new(Pool::new(
        thread::available_parallelism().map_or(16, |n| (n.get() * 4).clamp(8, 64)),
        thread::available_parallelism().map_or(128, |n| (n.get() * 16).clamp(128, 1024)),
    ))
});

/// RAII guard that decrements `alive_workers` when the worker thread
/// exits — covers both the "loop returned normally" path (currently
/// unreachable, but defended against future refactors) and the "thread
/// hard-aborted" path (panic during the queue lock acquisition or a
/// future change that lets the loop exit). Without this the counter
/// only ever monotonically increases, so an aborted worker would
/// permanently subtract one slot from the effective pool while the
/// `alive < max_workers` check still believes everyone is alive.
struct WorkerLifeTracker {
    pool: Arc<Pool>,
}

impl Drop for WorkerLifeTracker {
    fn drop(&mut self) {
        self.pool.alive_workers.fetch_sub(1, Ordering::AcqRel);
    }
}

fn worker_loop(pool: Arc<Pool>) {
    let _tracker = WorkerLifeTracker {
        pool: Arc::clone(&pool),
    };
    loop {
        let job = {
            let mut state = pool.state.lock();
            while state.queue.is_empty() && !state.shutting_down {
                pool.not_empty.wait(&mut state);
            }
            if state.shutting_down {
                return;
            }
            state.queue.pop_front().expect("queue was non-empty")
        };
        // Catch panics so a single bad job (plugin OS call, dialog handler)
        // doesn't kill the worker thread and silently inflate `alive_workers`
        // — over time that would degrade the pool to zero real workers while
        // the counter still reports max_workers. Log the panic so callers can
        // diagnose; a job's own catch_unwind wrapper (see `dispatch.rs` /
        // `scheme_bin.rs`) is still responsible for replying to the frontend
        // — this layer can't because it doesn't know id/label/proxy.
        if let Err(panic) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(job)) {
            let msg = panic
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| panic.downcast_ref::<&str>().map(|s| (*s).to_string()))
                .unwrap_or_else(|| "<non-string panic>".to_string());
            tracing::error!(target: "tynd::os::call_pool", "worker job panicked: {msg}");
        }
    }
}

/// Submit a job. Enqueues into a bounded queue and grows workers on demand up
/// to `max_workers`. Returns immediately when saturated so a caller on the UI
/// thread never blocks waiting for native work to drain.
pub(crate) fn submit<F: FnOnce() + Send + 'static>(f: F) -> Result<(), SubmitError> {
    submit_to(&POOL, f)
}

fn submit_to<F: FnOnce() + Send + 'static>(pool: &Arc<Pool>, f: F) -> Result<(), SubmitError> {
    let mut state = pool.state.lock();
    reap_finished(&mut state.joins);
    if state.shutting_down {
        return Err(SubmitError::ShuttingDown);
    }
    if state.queue.len() >= pool.max_queue {
        return Err(SubmitError::Saturated {
            capacity: pool.max_queue,
        });
    }
    state.queue.push_back(Box::new(f));
    if let Err(error) = grow_workers(pool, &mut state) {
        // No worker is alive to drain the queue: reclaim the just-pushed
        // job and fail explicitly instead of leaving it parked forever. The
        // job is dropped after the pool mutex is released.
        let rejected = state.queue.pop_back();
        drop(state);
        drop(rejected);
        return Err(SubmitError::ThreadSpawn(error));
    }
    drop(state);
    pool.not_empty.notify_one();
    Ok(())
}

/// Spawn one more worker unless `max_workers` are already alive. Loops the
/// CAS so a concurrent `WorkerLifeTracker` drop (which decrements
/// `alive_workers` outside the queue lock) can't make us skip the spawn and
/// leave a just-pushed job parked behind zero workers.
///
/// A failed spawn is only an error when no worker is left alive; otherwise
/// the existing workers drain the queue and the failure is logged.
fn grow_workers(pool: &Arc<Pool>, state: &mut PoolState) -> Result<(), std::io::Error> {
    loop {
        let alive = pool.alive_workers.load(Ordering::Acquire);
        if alive >= pool.max_workers {
            return Ok(());
        }
        if pool
            .alive_workers
            .compare_exchange(alive, alive + 1, Ordering::AcqRel, Ordering::Relaxed)
            .is_err()
        {
            continue;
        }
        let worker_pool = Arc::clone(pool);
        let spawned = thread::Builder::new()
            .name("tynd-os-worker".into())
            .spawn(move || worker_loop(worker_pool));
        return match spawned {
            Ok(join) => {
                state.joins.push(join);
                Ok(())
            },
            Err(error) => {
                pool.alive_workers.fetch_sub(1, Ordering::AcqRel);
                if pool.alive_workers.load(Ordering::Acquire) == 0 {
                    return Err(error);
                }
                tracing::warn!(
                    target: "tynd::os::call_pool",
                    "failed to grow OS worker pool: {error}"
                );
                Ok(())
            },
        };
    }
}

fn reap_finished(joins: &mut Vec<JoinHandle<()>>) {
    let mut index = 0;
    while index < joins.len() {
        if joins[index].is_finished() {
            let join = joins.swap_remove(index);
            let _ = join.join();
        } else {
            index += 1;
        }
    }
}

const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);

/// Close job admission, cancel work that has not started, wake parked workers,
/// and join workers that finish under one bounded deadline.
pub(crate) fn shutdown() {
    shutdown_pool(&POOL, SHUTDOWN_TIMEOUT);
}

fn shutdown_pool(pool: &Arc<Pool>, timeout: Duration) {
    let (mut joins, cancelled) = {
        let mut state = pool.state.lock();
        if state.shutting_down {
            return;
        }
        state.shutting_down = true;
        (
            std::mem::take(&mut state.joins),
            std::mem::take(&mut state.queue),
        )
    };
    pool.not_empty.notify_all();
    // Captured job values may have arbitrary Drop implementations. Destroy
    // them after releasing the pool mutex so a re-entrant submit cannot
    // deadlock teardown.
    drop(cancelled);

    let current_thread = thread::current().id();
    let deadline = Instant::now() + timeout;
    while !joins.is_empty() && Instant::now() < deadline {
        let mut index = 0;
        while index < joins.len() {
            if joins[index].thread().id() == current_thread {
                let _ = joins.swap_remove(index);
            } else if joins[index].is_finished() {
                let join = joins.swap_remove(index);
                let _ = join.join();
            } else {
                index += 1;
            }
        }
        if !joins.is_empty() {
            thread::sleep(Duration::from_millis(10));
        }
    }

    if !joins.is_empty() {
        crate::tynd_warn!(
            "OS call-pool shutdown deadline elapsed with {} running worker(s)",
            joins.len()
        );
    }
}
