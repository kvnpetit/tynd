#[cfg(feature = "plugins")]
use std::{panic, sync::mpsc, thread::JoinHandle};
use std::{
    path::PathBuf,
    sync::OnceLock,
    thread::{self, ThreadId},
    time::Duration,
};

/// Global cleanup registry: paths to delete before any process::exit, plus
/// the optional shutdown callback that signals the running event loop.
///
/// `std::process::exit` bypasses Rust's drop machinery, so `Drop`-based cleanup
/// (e.g. `tempfile::TempDir`) never runs. Instead, callers register paths here
/// and `app::run_app` calls `cleanup::run()` before every exit point.
///
/// The shutdown callback is registered by `app::run_app` after the event loop
/// proxy exists; signal handlers (`init_signals()`) invoke it to send a quit
/// `UserEvent` so the loop unwinds cleanly instead of dying mid-frame.
use parking_lot::{Condvar, Mutex};

static DIRS: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

#[derive(Debug)]
enum CleanupStatus {
    Pending,
    Running(ThreadId),
    Complete,
}

/// Serializes every graceful-exit path. Concurrent callers wait for the owner;
/// a reentrant call from a plugin shutdown hook returns immediately.
struct CleanupGate {
    status: Mutex<CleanupStatus>,
    complete: Condvar,
}

impl CleanupGate {
    const fn new() -> Self {
        Self {
            status: Mutex::new(CleanupStatus::Pending),
            complete: Condvar::new(),
        }
    }

    fn run(&self, cleanup: impl FnOnce()) {
        let current_thread = thread::current().id();
        let mut status = self.status.lock();
        loop {
            match *status {
                CleanupStatus::Pending => {
                    *status = CleanupStatus::Running(current_thread);
                    break;
                },
                CleanupStatus::Running(owner) if owner == current_thread => return,
                CleanupStatus::Running(_) => self.complete.wait(&mut status),
                CleanupStatus::Complete => return,
            }
        }
        drop(status);

        let _completion = crate::util::defer(|| {
            let mut status = self.status.lock();
            *status = CleanupStatus::Complete;
            self.complete.notify_all();
        });
        cleanup();
    }
}

static CLEANUP_GATE: CleanupGate = CleanupGate::new();

#[cfg(feature = "plugins")]
const PLUGIN_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);

/// Closure registered by `app::run_app` so signal handlers can ask the
/// event loop to exit cleanly. `Send + Sync` for arbitrary handler threads.
type ShutdownHook = Box<dyn Fn() + Send + Sync + 'static>;
static SHUTDOWN_HOOK: OnceLock<ShutdownHook> = OnceLock::new();

/// Register a directory to be deleted before the process exits.
pub(crate) fn register_dir(path: PathBuf) {
    DIRS.lock().push(path);
}

/// Register the signal-layer shutdown hook. Idempotent.
pub(crate) fn set_shutdown_hook<F>(hook: F)
where
    F: Fn() + Send + Sync + 'static,
{
    let _ = SHUTDOWN_HOOK.set(Box::new(hook));
}

/// Invoke the registered shutdown hook, if any. Used by the signal handler
/// to ask the event loop to exit gracefully. Returns `true` if a hook ran.
pub(crate) fn trigger_shutdown() -> bool {
    if let Some(hook) = SHUTDOWN_HOOK.get() {
        hook();
        true
    } else {
        false
    }
}

/// Run plugin shutdown hooks then delete every registered directory. Called
/// by `run_app` before exit, by signal handling, and by the runtime's
/// outermost uncaught-panic guard.
pub(crate) fn run() {
    CLEANUP_GATE.run(run_once);
}

fn run_once() {
    let _complete_log = crate::util::defer(|| {
        tracing::debug!(target: "tynd::cleanup", "cleanup::run complete");
    });

    // Deliver queued Sentry events: `process::exit` never runs the guard's Drop.
    crate::observability::flush(Duration::from_secs(2));

    #[cfg(feature = "plugins")]
    {
        // Plugins flush state before we delete the temp dirs they may write to.
        let timed_out = run_bounded_task("native-plugin shutdown", PLUGIN_SHUTDOWN_TIMEOUT, || {
            crate::plugins::shutdown_all();
        })
        .is_some();
        if !timed_out {
            // Plugin threads may hold a cloned `EmitFn` and still be writing a
            // final event when shutdown_all returns.
            thread::sleep(Duration::from_millis(100));
        }
    }

    // Stop long-lived OS resources after plugins have had a chance to release
    // them explicitly. Process shutdown closes spawn admission before draining,
    // so an in-flight dispatch cannot create an orphan after the snapshot.
    crate::api::shutdown_resources();

    // Drop the debug-protocol discovery file so MCP clients don't hit a
    // stale `<pid>.json` after a crash. No-op when the debug protocol
    // wasn't activated. Gated on the feature so custom builds without
    // it skip the call entirely.
    #[cfg(feature = "debug-protocol")]
    crate::debug::cleanup_discovery_file();

    // Snapshot + drain under the lock; do the deletes outside so concurrent
    // `register_dir` calls don't block on the IO window.
    let dirs: Vec<PathBuf> = {
        let mut guard = DIRS.lock();
        let snapshot = guard.clone();
        guard.clear();
        snapshot
    };
    for path in dirs {
        let _ = std::fs::remove_dir_all(path);
    }
}

/// Run potentially untrusted teardown code without letting it hold the host's
/// global cleanup gate forever. Rust cannot safely kill a stuck thread; on
/// timeout the worker is detached and process exit finishes reclaiming it.
#[cfg(feature = "plugins")]
fn run_bounded_task(
    name: &'static str,
    timeout: Duration,
    task: impl FnOnce() + Send + 'static,
) -> Option<JoinHandle<()>> {
    let (done_tx, done_rx) = mpsc::sync_channel(1);
    let worker = match thread::Builder::new()
        .name("tynd-bounded-cleanup".to_string())
        .spawn(move || {
            let completed = panic::catch_unwind(panic::AssertUnwindSafe(task)).is_ok();
            let _ = done_tx.send(completed);
        }) {
        Ok(worker) => worker,
        Err(error) => {
            tracing::error!(
                target: "tynd::cleanup",
                task = name,
                %error,
                "failed to start bounded cleanup worker"
            );
            return None;
        },
    };

    match done_rx.recv_timeout(timeout) {
        Ok(true) => None,
        Ok(false) => {
            tracing::error!(
                target: "tynd::cleanup",
                task = name,
                "bounded cleanup task panicked"
            );
            None
        },
        Err(mpsc::RecvTimeoutError::Timeout) => {
            tracing::error!(
                target: "tynd::cleanup",
                task = name,
                timeout_ms = timeout.as_millis(),
                "bounded cleanup task timed out; worker detached"
            );
            Some(worker)
        },
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            tracing::error!(
                target: "tynd::cleanup",
                task = name,
                "bounded cleanup worker disconnected"
            );
            None
        },
    }
}
