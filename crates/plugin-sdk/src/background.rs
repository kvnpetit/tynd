use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use parking_lot::Mutex;

/// Drop stops the loop and joins; [`stop`] / [`join`] provide finer shutdown.
///
/// [`stop`]: BackgroundHandle::stop
/// [`join`]: BackgroundHandle::join
pub struct BackgroundHandle {
    pub(crate) running: Arc<AtomicBool>,
    pub(crate) stop_tx: crossbeam_channel::Sender<()>,
    /// `Mutex` keeps the handle `Send + Sync` so plugins can store it in a map.
    pub(crate) join: Mutex<Option<JoinHandle<()>>>,
    /// How long [`Drop`] waits for the worker thread to finish before detaching it.
    pub(crate) drop_timeout: Duration,
}

impl BackgroundHandle {
    pub(crate) fn new(
        running: Arc<AtomicBool>,
        stop_tx: crossbeam_channel::Sender<()>,
        join: Option<JoinHandle<()>>,
    ) -> Self {
        Self {
            running,
            stop_tx,
            join: Mutex::new(join),
            drop_timeout: BACKGROUND_HANDLE_DROP_TIMEOUT,
        }
    }

    /// Signal the loop to exit. Non-blocking; cancellation latency is bounded by the in-flight
    /// body.
    pub fn stop(&self) {
        self.running.store(false, Ordering::Release);
        let _ = self.stop_tx.try_send(());
    }

    /// Whether the loop has seen the stop signal yet.
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::Acquire)
    }

    /// Stop the loop and block until the worker thread finishes. Idempotent.
    pub fn join(&self) {
        self.stop();
        if let Some(handle) = self.join.lock().take() {
            let _ = handle.join();
        }
    }

    /// Override the [`Drop`] join-timeout.
    #[must_use]
    pub fn with_drop_timeout(mut self, timeout: Duration) -> Self {
        self.drop_timeout = timeout;
        self
    }
}

/// Default upper bound on how long [`BackgroundHandle::drop`] waits for the worker thread.
pub const BACKGROUND_HANDLE_DROP_TIMEOUT: Duration = Duration::from_secs(2);

impl Drop for BackgroundHandle {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Release);
        let _ = self.stop_tx.try_send(());
        let handle = self.join.lock().take();
        if let Some(handle) = handle {
            let (tx, rx) = std::sync::mpsc::channel();
            match thread::Builder::new()
                .name("tynd-plugin-background-join".to_string())
                .spawn(move || {
                    let _ = handle.join();
                    let _ = tx.send(());
                }) {
                Ok(_) => {
                    if rx.recv_timeout(self.drop_timeout).is_err() {
                        tracing::warn!(
                            timeout_secs = self.drop_timeout.as_secs(),
                            "BackgroundHandle: join timed out - detaching worker thread AND the \
                             helper join thread (both leak until process exit). Call stop()+join() \
                             with your own deadline to avoid this.",
                        );
                    }
                },
                Err(error) => {
                    tracing::warn!(
                        %error,
                        "BackgroundHandle: could not start bounded join helper; worker detached",
                    );
                },
            }
        }
    }
}

impl std::fmt::Debug for BackgroundHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BackgroundHandle")
            .field("running", &self.is_running())
            .finish_non_exhaustive()
    }
}
