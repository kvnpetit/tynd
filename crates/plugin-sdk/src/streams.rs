//! Gated, bounded, cancellable background streams ("reserve, start, chunks,
//! end") and a bounded thread join.
//!
//! The protocol every long-running streaming command follows:
//!
//! 1. the command *reserves* a stream ([`GatedStreams::spawn_stream`]): it
//!    takes a permit (bounded wait), registers a cancel flag and a start gate
//!    and spawns a worker that parks on the gate, then returns the stream id;
//! 2. the client subscribes to the plugin's events and calls `startStream`
//!    ([`GatedStreams::start`]) which opens the gate: no chunk is emitted before
//!    the listener exists;
//! 3. the worker emits chunk events on its own, then exactly one `end_event`
//!    (`{ stream, ok, error? }`).
//!
//! What the helper guarantees, so plugins do not each re-implement it:
//!
//! * permit acquisition times out ("busy") instead of parking a shared
//!   call-pool worker forever;
//! * a reserved stream whose `startStream` never arrives (page reload) expires:
//!   the permit is freed and `end { ok: false }` is emitted;
//! * a worker error or panic becomes `end { ok: false, error }`;
//! * [`GatedStreams::cancel`] works before and after the start;
//! * [`GatedStreams::shutdown`] cancels everything, wakes gated workers and
//!   waits (bounded) for them. Call it from your plugin's `shutdown()`: plugin
//!   structs are never dropped, so `Drop` is not a teardown hook.

use std::{
    panic::{catch_unwind, AssertUnwindSafe},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

use crossbeam_channel::{bounded, RecvTimeoutError, Sender};
use dashmap::DashMap;
use serde_json::json;

use crate::{sanitize_event_string, BoundedPermitPool, EmitFn, Permit, PluginError, PluginResult};

/// Default wait for a free slot before a call fails with "busy".
pub const DEFAULT_PERMIT_WAIT: Duration = Duration::from_secs(15);
/// Default wait for `startStream` before a reserved stream expires.
pub const DEFAULT_START_WAIT: Duration = Duration::from_secs(30);
/// Suggested upper bound for [`GatedStreams::shutdown`].
pub const DEFAULT_SHUTDOWN_DEADLINE: Duration = Duration::from_secs(5);

/// Timeouts of a [`GatedStreams`], injectable so tests do not wait for real
/// durations.
#[derive(Debug, Clone, Copy)]
pub struct GatedLimits {
    /// How long a call waits for a free slot before failing with "busy".
    pub permit_wait: Duration,
    /// How long a reserved stream waits for `startStream` before it expires.
    pub start_wait: Duration,
}

impl Default for GatedLimits {
    fn default() -> Self {
        Self {
            permit_wait: DEFAULT_PERMIT_WAIT,
            start_wait: DEFAULT_START_WAIT,
        }
    }
}

/// Removes a synchronous run's cancel flag when the run ends (or panics).
struct SyncGuard<'a> {
    streams: &'a GatedStreams,
    id: u64,
}

impl Drop for SyncGuard<'_> {
    fn drop(&mut self) {
        self.streams.cancels.remove(&self.id);
    }
}

/// Bounded, cancellable, gated background streams. See the [module
/// docs](self) for the protocol.
#[derive(Debug)]
pub struct GatedStreams {
    pool: BoundedPermitPool,
    /// Cancel flag per live stream (and per synchronous run).
    cancels: DashMap<u64, Arc<AtomicBool>>,
    /// One-shot start gates of reserved streams that have not started yet.
    starts: DashMap<u64, Sender<()>>,
    next_id: AtomicU64,
    closed: AtomicBool,
    limits: GatedLimits,
}

impl GatedStreams {
    /// `capacity` concurrent slots (clamped to at least 1). `pool_name` shows
    /// up in the "busy" error: `"{operation}: {pool_name} busy (waited Ns) -
    /// retry later"`.
    pub fn new(capacity: usize, pool_name: &str, limits: GatedLimits) -> Self {
        Self {
            pool: BoundedPermitPool::new(capacity.max(1), pool_name),
            cancels: DashMap::new(),
            starts: DashMap::new(),
            next_id: AtomicU64::new(1),
            closed: AtomicBool::new(false),
            limits,
        }
    }

    /// Free slots right now.
    pub fn available(&self) -> usize {
        self.pool.available()
    }

    /// Streams currently registered (running, waiting for their start, or
    /// inside [`run_sync`](Self::run_sync)).
    pub fn live(&self) -> usize {
        self.cancels.len()
    }

    fn shutting_down(operation: &str) -> PluginError {
        PluginError::Custom(format!("{operation}: plugin is shutting down"))
    }

    fn acquire(&self, operation: &str) -> PluginResult<Permit> {
        if self.closed.load(Ordering::SeqCst) {
            return Err(Self::shutting_down(operation));
        }
        self.pool
            .acquire_timeout(operation, self.limits.permit_wait)
    }

    /// Run `work` on the calling thread inside a permit. `work` receives a
    /// cancel flag that [`shutdown`](Self::shutdown) flips; when it was flipped
    /// the run fails instead of returning a partial result.
    pub fn run_sync<R>(
        &self,
        operation: &str,
        work: impl FnOnce(&AtomicBool) -> PluginResult<R>,
    ) -> PluginResult<R> {
        let _permit = self.acquire(operation)?;
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let cancel = Arc::new(AtomicBool::new(false));
        self.cancels.insert(id, Arc::clone(&cancel));
        let _guard = SyncGuard { streams: self, id };
        if self.closed.load(Ordering::SeqCst) {
            return Err(Self::shutting_down(operation));
        }
        let out = work(&cancel)?;
        if cancel.load(Ordering::Acquire) {
            return Err(PluginError::Custom(format!(
                "{operation}: cancelled by plugin shutdown"
            )));
        }
        Ok(out)
    }

    /// Reserve a stream: acquire a permit, register its cancel flag and start
    /// gate and spawn the worker, which parks until [`start`](Self::start).
    /// The worker emits `end_event` once when it ran, failed or expired, and
    /// `ok: true` when it was cancelled before starting. A stream whose gate
    /// is not opened within the start timeout expires: the permit is released
    /// and `end { ok: false }` is emitted. A panic in `run` becomes
    /// `end { ok: false }`.
    pub fn spawn_stream(
        self: &Arc<Self>,
        operation: &'static str,
        thread_prefix: &str,
        emit: EmitFn,
        end_event: &'static str,
        run: impl FnOnce(u64, &AtomicBool) -> PluginResult<()> + Send + 'static,
    ) -> PluginResult<u64> {
        let permit = self.acquire(operation)?;
        let stream_id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let cancel = Arc::new(AtomicBool::new(false));
        let (start_tx, start_rx) = bounded::<()>(1);
        self.cancels.insert(stream_id, Arc::clone(&cancel));
        self.starts.insert(stream_id, start_tx);

        // `shutdown` sets `closed` before it sweeps the maps: either the sweep
        // sees the entries inserted above or this re-check sees `closed`.
        // SeqCst on both sides: this store/load handshake needs a total order
        // (Release/Acquire would allow both sides to miss each other on weak
        // memory models).
        if self.closed.load(Ordering::SeqCst) {
            self.forget(stream_id);
            return Err(Self::shutting_down(operation));
        }

        let streams = Arc::clone(self);
        let start_wait = self.limits.start_wait;
        let spawned = std::thread::Builder::new()
            .name(format!("{thread_prefix}-{stream_id}"))
            .spawn(move || {
                // The permit lives as long as the worker, whatever the exit path.
                let _permit = permit;
                match start_rx.recv_timeout(start_wait) {
                    Ok(()) => {},
                    Err(RecvTimeoutError::Disconnected) => {
                        // `cancel` / `shutdown` dropped the gate.
                        streams.forget(stream_id);
                        if cancel.load(Ordering::Acquire) {
                            emit(end_event, json!({ "stream": stream_id, "ok": true }));
                        }
                        return;
                    },
                    Err(RecvTimeoutError::Timeout) => {
                        // Never started: free the permit and tell the client.
                        streams.forget(stream_id);
                        emit(
                            end_event,
                            json!({
                                "stream": stream_id,
                                "ok": false,
                                "error": "stream expired: startStream was not called in time",
                            }),
                        );
                        return;
                    },
                }
                let outcome = catch_unwind(AssertUnwindSafe(|| run(stream_id, &cancel)));
                streams.forget(stream_id);
                let result = outcome.unwrap_or_else(|_| {
                    Err(PluginError::Custom(format!("{operation}: worker panicked")))
                });
                match result {
                    Ok(()) => emit(end_event, json!({ "stream": stream_id, "ok": true })),
                    Err(error) => emit(
                        end_event,
                        json!({
                            "stream": stream_id,
                            "ok": false,
                            "error": sanitize_event_string(&error.to_string()),
                        }),
                    ),
                }
            });
        if let Err(error) = spawned {
            self.forget(stream_id);
            return Err(PluginError::Custom(format!(
                "{operation}: failed to start worker: {error}"
            )));
        }
        Ok(stream_id)
    }

    /// Release a reserved stream once the client listens for its events.
    /// Idempotent for unknown / already expired ids.
    pub fn start(&self, operation: &str, stream_id: u64) -> PluginResult<()> {
        let Some((_, start)) = self.starts.remove(&stream_id) else {
            return Ok(());
        };
        start.send(()).map_err(|error| {
            self.cancels.remove(&stream_id);
            PluginError::Custom(format!(
                "{operation}: worker {stream_id} is unavailable: {error}"
            ))
        })
    }

    /// Flag a stream as cancelled. A stream that never started is woken
    /// without running its work. Idempotent.
    pub fn cancel(&self, stream_id: u64) {
        if let Some(flag) = self.cancels.get(&stream_id) {
            flag.store(true, Ordering::Release);
        }
        self.starts.remove(&stream_id);
    }

    /// Cancel everything, refuse new work and wait up to `deadline` for the
    /// workers to drain. Returns `false` when some work is still running
    /// (native code cannot always be interrupted mid-call).
    pub fn shutdown(&self, deadline: Duration) -> bool {
        self.closed.store(true, Ordering::SeqCst);
        for entry in &self.cancels {
            entry.value().store(true, Ordering::Release);
        }
        // Dropping the senders wakes every gated worker.
        self.starts.clear();
        self.pool.wait_idle(deadline)
    }

    fn forget(&self, stream_id: u64) {
        self.starts.remove(&stream_id);
        self.cancels.remove(&stream_id);
    }
}

/// Result of [`join_with_timeout`].
#[derive(Debug)]
pub enum JoinOutcome<T> {
    /// The thread finished and returned a value.
    Finished(T),
    /// The thread finished by panicking.
    Panicked,
    /// The deadline passed first; the handle is returned so the caller can
    /// keep it, retry or detach.
    TimedOut(JoinHandle<T>),
}

/// Wait up to `timeout` for `handle` to finish, then join it. Never blocks
/// past the timeout and never spawns a helper thread (it polls
/// [`JoinHandle::is_finished`] with a short, growing sleep), so a stuck worker
/// leaks nothing but itself.
///
/// To share one deadline across several threads, compute the remaining time
/// before each call: `deadline.saturating_duration_since(Instant::now())`.
pub fn join_with_timeout<T>(handle: JoinHandle<T>, timeout: Duration) -> JoinOutcome<T> {
    let deadline = Instant::now().checked_add(timeout);
    let mut pause = Duration::from_millis(1);
    while !handle.is_finished() {
        let remaining = deadline.map(|deadline| deadline.saturating_duration_since(Instant::now()));
        if remaining.is_some_and(|remaining| remaining.is_zero()) {
            return JoinOutcome::TimedOut(handle);
        }
        std::thread::sleep(remaining.map_or(pause, |remaining| pause.min(remaining)));
        pause = (pause * 2).min(Duration::from_millis(10));
    }
    match handle.join() {
        Ok(value) => JoinOutcome::Finished(value),
        Err(_) => JoinOutcome::Panicked,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use super::*;
    use crate::test_cx;

    fn quick(permit_ms: u64, start_ms: u64) -> GatedLimits {
        GatedLimits {
            permit_wait: Duration::from_millis(permit_ms),
            start_wait: Duration::from_millis(start_ms),
        }
    }

    fn streams(capacity: usize, permit_ms: u64, start_ms: u64) -> GatedStreams {
        GatedStreams::new(capacity, "test slots", quick(permit_ms, start_ms))
    }

    fn wait_for(mut condition: impl FnMut() -> bool) -> bool {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if condition() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        false
    }

    #[test]
    fn acquire_times_out_instead_of_blocking_forever() {
        let streams = streams(1, 40, 1000);
        let held = streams.acquire("test").unwrap();
        let started = Instant::now();
        let error = streams.acquire("test").unwrap_err().to_string();
        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(error.contains("busy"), "{error}");
        drop(held);
        assert!(streams.acquire("test").is_ok());
    }

    #[test]
    fn unstarted_stream_expires_and_releases_its_permit() {
        let streams = Arc::new(streams(1, 500, 30));
        let (cx, capture) = test_cx();
        let id = streams
            .spawn_stream("test.stream", "t", cx.emitter(), "t:end", |_, _| Ok(()))
            .unwrap();
        assert_eq!(streams.available(), 0);
        assert!(wait_for(|| streams.available() == 1));
        assert_eq!(streams.live(), 0);
        let events = capture.events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].0, "t:end");
        assert_eq!(events[0].1["stream"], id);
        assert_eq!(events[0].1["ok"], false);
        // A late startStream on the expired id is a harmless no-op.
        assert!(streams.start("test.start", id).is_ok());
    }

    #[test]
    fn started_stream_runs_and_ends_ok() {
        let streams = Arc::new(streams(1, 500, 5000));
        let (cx, capture) = test_cx();
        let id = streams
            .spawn_stream("test.stream", "t", cx.emitter(), "t:end", |_, cancel| {
                assert!(!cancel.load(Ordering::Acquire));
                Ok(())
            })
            .unwrap();
        streams.start("test.start", id).unwrap();
        assert!(wait_for(|| !capture.events().is_empty()));
        assert_eq!(capture.events()[0].1["ok"], true);
        assert!(wait_for(|| streams.available() == 1 && streams.live() == 0));
    }

    #[test]
    fn worker_error_and_panic_become_failed_end_events() {
        let streams = Arc::new(streams(2, 500, 5000));
        let (cx, capture) = test_cx();
        let failing = streams
            .spawn_stream("test.stream", "t", cx.emitter(), "t:end", |_, _| {
                Err(PluginError::Custom("boom".into()))
            })
            .unwrap();
        let panicking = streams
            .spawn_stream("test.stream", "t", cx.emitter(), "t:end", |_, _| {
                panic!("worker bug")
            })
            .unwrap();
        streams.start("s", failing).unwrap();
        streams.start("s", panicking).unwrap();
        assert!(wait_for(|| capture.events().len() == 2));
        assert!(capture.events().iter().all(|(_, p)| p["ok"] == false));
        assert!(wait_for(|| streams.available() == 2));
    }

    #[test]
    fn cancel_before_start_wakes_the_worker_without_running_it() {
        let streams = Arc::new(streams(1, 500, 5000));
        let (cx, capture) = test_cx();
        let ran = Arc::new(AtomicBool::new(false));
        let ran_in_worker = Arc::clone(&ran);
        let id = streams
            .spawn_stream("test.stream", "t", cx.emitter(), "t:end", move |_, _| {
                ran_in_worker.store(true, Ordering::Release);
                Ok(())
            })
            .unwrap();
        streams.cancel(id);
        assert!(wait_for(|| streams.available() == 1));
        assert!(!ran.load(Ordering::Acquire));
        assert!(wait_for(|| capture.events().len() == 1));
        assert_eq!(capture.events()[0].1["ok"], true);
    }

    #[test]
    fn shutdown_cancels_running_streams_and_drains() {
        let streams = Arc::new(streams(2, 500, 5000));
        let (cx, capture) = test_cx();
        let (running_tx, running_rx) = mpsc::channel();
        let id = streams
            .spawn_stream(
                "test.stream",
                "t",
                cx.emitter(),
                "t:end",
                move |_, cancel| {
                    running_tx.send(()).unwrap();
                    while !cancel.load(Ordering::Acquire) {
                        std::thread::sleep(Duration::from_millis(2));
                    }
                    Ok(())
                },
            )
            .unwrap();
        // A second stream still parked on its start gate.
        streams
            .spawn_stream("test.stream", "t", cx.emitter(), "t:end", |_, _| Ok(()))
            .unwrap();
        streams.start("s", id).unwrap();
        running_rx.recv_timeout(Duration::from_secs(5)).unwrap();

        assert!(streams.shutdown(Duration::from_secs(5)));
        assert_eq!(streams.available(), 2);
        assert_eq!(streams.live(), 0);
        assert!(wait_for(|| capture.events().len() == 2));
        // New work is refused once shut down.
        let refused = streams
            .spawn_stream("test.stream", "t", cx.emitter(), "t:end", |_, _| Ok(()))
            .unwrap_err()
            .to_string();
        assert!(refused.contains("shutting down"), "{refused}");
        assert!(streams.run_sync("test.sync", |_| Ok(())).is_err());
    }

    #[test]
    fn shutdown_reports_work_that_does_not_stop_in_time() {
        let streams = Arc::new(streams(1, 500, 5000));
        let (cx, _capture) = test_cx();
        let (running_tx, running_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel::<()>();
        let id = streams
            .spawn_stream("test.stream", "t", cx.emitter(), "t:end", move |_, _| {
                running_tx.send(()).unwrap();
                // Ignores the cancel flag, like a mid-decode native call.
                let _ = release_rx.recv();
                Ok(())
            })
            .unwrap();
        streams.start("s", id).unwrap();
        running_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(!streams.shutdown(Duration::from_millis(30)));
        release_tx.send(()).unwrap();
        assert!(wait_for(|| streams.available() == 1));
    }

    #[test]
    fn run_sync_holds_a_permit_and_cleans_up() {
        let streams = streams(1, 30, 1000);
        let out = streams
            .run_sync("test.sync", |_| {
                assert_eq!(streams.available(), 0);
                assert_eq!(streams.live(), 1);
                // Pool exhausted: a nested call must time out, not deadlock.
                assert!(streams.acquire("nested").is_err());
                Ok(7)
            })
            .unwrap();
        assert_eq!(out, 7);
        assert_eq!(streams.available(), 1);
        assert_eq!(streams.live(), 0);
        let failed: PluginResult<()> =
            streams.run_sync("test.sync", |_| Err(PluginError::Custom("nope".into())));
        assert!(failed.is_err());
        assert_eq!(streams.available(), 1);
    }

    #[test]
    fn join_with_timeout_reports_finished_panicked_and_timed_out() {
        let done = std::thread::spawn(|| 5_u32);
        assert!(matches!(
            join_with_timeout(done, Duration::from_secs(5)),
            JoinOutcome::Finished(5)
        ));

        let panicking = std::thread::Builder::new()
            .name("t-panic".into())
            .spawn(|| -> u32 { panic!("expected test panic") })
            .unwrap();
        assert!(matches!(
            join_with_timeout(panicking, Duration::from_secs(5)),
            JoinOutcome::Panicked
        ));

        let (release_tx, release_rx) = mpsc::channel::<()>();
        let stuck = std::thread::spawn(move || {
            let _ = release_rx.recv();
        });
        let started = Instant::now();
        let handle = match join_with_timeout(stuck, Duration::from_millis(40)) {
            JoinOutcome::TimedOut(handle) => handle,
            other => panic!("expected a timeout, got {other:?}"),
        };
        assert!(started.elapsed() < Duration::from_secs(2));
        // The returned handle is still joinable once the thread is released.
        release_tx.send(()).unwrap();
        assert!(matches!(
            join_with_timeout(handle, Duration::from_secs(5)),
            JoinOutcome::Finished(())
        ));
    }
}
