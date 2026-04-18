//! setTimeout / setInterval scheduler.
//!
//! A single OS thread maintains a min-heap ordered by fire_at. `recv_timeout`
//! parks it until either the next deadline or a new command arrives — no
//! busy-wait, and no separate thread per timer. Cancellation is lazy:
//! cleared IDs are marked and skipped when they reach the top of the heap.

use std::collections::{BinaryHeap, HashSet};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use super::JsMsg;

pub(super) enum TimerCmd {
    Set { id: u32, ms: u32, once: bool },
    Clear(u32),
}

pub(super) fn start_timer_thread(cmd_rx: mpsc::Receiver<TimerCmd>, js_tx: mpsc::Sender<JsMsg>) {
    std::thread::spawn(move || {
        #[derive(Eq, PartialEq)]
        struct Entry {
            fire_at: Instant,
            id: u32,
            ms: u32,
            once: bool,
        }

        // Reverse Ord so BinaryHeap (max-heap) behaves as a min-heap on fire_at.
        impl Ord for Entry {
            fn cmp(&self, o: &Self) -> std::cmp::Ordering {
                o.fire_at.cmp(&self.fire_at)
            }
        }
        impl PartialOrd for Entry {
            fn partial_cmp(&self, o: &Self) -> Option<std::cmp::Ordering> {
                Some(self.cmp(o))
            }
        }

        let mut heap: BinaryHeap<Entry> = BinaryHeap::with_capacity(32);
        let mut cancelled: HashSet<u32> = HashSet::new();
        let mut active: HashSet<u32> = HashSet::new();

        loop {
            let now = Instant::now();

            while heap.peek().is_some_and(|e| e.fire_at <= now) {
                let e = heap.pop().unwrap();
                if cancelled.contains(&e.id) {
                    cancelled.remove(&e.id);
                    active.remove(&e.id);
                    continue;
                }
                if js_tx.send(JsMsg::TimerFire(e.id)).is_err() {
                    return;
                }
                if e.once {
                    active.remove(&e.id);
                } else {
                    // Re-schedule interval using fire_at + dur to prevent drift.
                    let dur = Duration::from_millis(u64::from(e.ms));
                    heap.push(Entry {
                        fire_at: e.fire_at + dur,
                        id: e.id,
                        ms: e.ms,
                        once: false,
                    });
                }
            }

            let timeout = heap.peek().map_or(Duration::from_secs(3600), |e| {
                e.fire_at.saturating_duration_since(Instant::now())
            });

            match cmd_rx.recv_timeout(timeout) {
                Ok(TimerCmd::Set { id, ms, once }) => {
                    let fire_at = Instant::now() + Duration::from_millis(u64::from(ms));
                    if active.contains(&id) {
                        cancelled.insert(id);
                    } else {
                        active.insert(id);
                    }
                    heap.push(Entry {
                        fire_at,
                        id,
                        ms,
                        once,
                    });
                },
                Ok(TimerCmd::Clear(id)) => {
                    if active.contains(&id) {
                        cancelled.insert(id);
                    }
                },
                Err(mpsc::RecvTimeoutError::Timeout) => {},
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
            }
        }
    });
}
