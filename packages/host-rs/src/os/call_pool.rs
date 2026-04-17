//! Bounded worker pool for OS-API dispatch.
//!
//! Every non-window OS call used to `std::thread::spawn` — spamming dialogs or
//! shell commands could therefore balloon the thread count. This pool caps the
//! steady-state threads at `available_parallelism * 4` (clamped to [8, 64])
//! while keeping the semantics: dialogs block on user input, so we size for
//! blocked workers, not CPU-bound throughput.
//!
//! Overflow policy: if the queue depth exceeds the worker count by more than
//! 2x, the submitter spawns a one-shot thread instead of enqueuing. That caps
//! latency for bursts without unbounded thread growth.

use std::collections::VecDeque;
use std::sync::{Condvar, Mutex, OnceLock};
use std::thread;

type Job = Box<dyn FnOnce() + Send + 'static>;

struct Pool {
    queue: Mutex<VecDeque<Job>>,
    not_empty: Condvar,
    worker_count: usize,
}

fn pool() -> &'static Pool {
    static POOL: OnceLock<Pool> = OnceLock::new();
    POOL.get_or_init(|| {
        let worker_count =
            thread::available_parallelism().map_or(16, |n| (n.get() * 4).clamp(8, 64));
        let pool = Pool {
            queue: Mutex::new(VecDeque::with_capacity(worker_count * 2)),
            not_empty: Condvar::new(),
            worker_count,
        };
        for _ in 0..worker_count {
            thread::spawn(worker_loop);
        }
        pool
    })
}

fn worker_loop() {
    let p = pool();
    loop {
        let job = {
            let mut q = p.queue.lock().expect("call_pool queue poisoned");
            while q.is_empty() {
                q = p.not_empty.wait(q).expect("call_pool condvar poisoned");
            }
            q.pop_front().expect("queue was non-empty")
        };
        job();
    }
}

/// Submit a job. Enqueues when the pool has headroom; overflows to a one-shot
/// thread once the backlog exceeds `worker_count * 2` so bursts don't starve
/// urgent calls behind a row of blocked dialogs.
pub fn submit<F: FnOnce() + Send + 'static>(f: F) {
    let p = pool();
    let overflow_threshold = p.worker_count.saturating_mul(2);
    let mut q = p.queue.lock().expect("call_pool queue poisoned");
    if q.len() >= overflow_threshold {
        drop(q);
        thread::spawn(f);
        return;
    }
    q.push_back(Box::new(f));
    drop(q);
    p.not_empty.notify_one();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    #[test]
    fn runs_submitted_jobs() {
        let counter = Arc::new(AtomicUsize::new(0));
        for _ in 0..50 {
            let c = counter.clone();
            submit(move || {
                c.fetch_add(1, Ordering::Relaxed);
            });
        }
        let deadline = Instant::now() + Duration::from_secs(2);
        while counter.load(Ordering::Relaxed) < 50 && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(counter.load(Ordering::Relaxed), 50);
    }

    #[test]
    fn slow_jobs_do_not_block_fast_ones_forever() {
        // Submit more blocking jobs than workers, then a burst of fast jobs:
        // overflow policy must let them through.
        let fast_done = Arc::new(AtomicUsize::new(0));
        for _ in 0..200 {
            submit(|| thread::sleep(Duration::from_millis(50)));
        }
        for _ in 0..20 {
            let d = fast_done.clone();
            submit(move || {
                d.fetch_add(1, Ordering::Relaxed);
            });
        }
        let deadline = Instant::now() + Duration::from_secs(3);
        while fast_done.load(Ordering::Relaxed) < 20 && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(fast_done.load(Ordering::Relaxed), 20);
    }
}
