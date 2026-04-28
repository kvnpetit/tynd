use std::{path::PathBuf, sync::OnceLock};

/// Global cleanup registry — paths to delete before any process::exit, plus
/// the optional shutdown callback that signals the running event loop.
///
/// `std::process::exit` bypasses Rust's drop machinery, so `Drop`-based cleanup
/// (e.g. `tempfile::TempDir`) never runs. Instead, callers register paths here
/// and `app::run_app` calls `cleanup::run()` before every exit point.
///
/// The shutdown callback is registered by `app::run_app` after the event loop
/// proxy exists; signal handlers (`init_signals()`) invoke it to send a quit
/// `UserEvent` so the loop unwinds cleanly instead of dying mid-frame.
use parking_lot::Mutex;

static DIRS: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

/// Closure registered by `app::run_app` to nudge the event loop into a clean
/// exit. We keep it as a `OnceLock` so registration is lock-free after the
/// first set; the value is `Send + Sync` because it's invoked from arbitrary
/// signal-handler threads.
type ShutdownHook = Box<dyn Fn() + Send + Sync + 'static>;
static SHUTDOWN_HOOK: OnceLock<ShutdownHook> = OnceLock::new();

/// Register a directory to be deleted before the process exits.
pub fn register_dir(path: PathBuf) {
    DIRS.lock().push(path);
}

/// Register the shutdown hook the signal layer should call when SIGINT /
/// SIGTERM fires. Idempotent — extra calls after the first are silently
/// ignored so `run_app` can be re-entered (rare, but tests do it).
pub fn set_shutdown_hook<F>(hook: F)
where
    F: Fn() + Send + Sync + 'static,
{
    let _ = SHUTDOWN_HOOK.set(Box::new(hook));
}

/// Invoke the registered shutdown hook, if any. Used by the signal handler
/// to ask the event loop to exit gracefully. Returns `true` if a hook ran.
pub fn trigger_shutdown() -> bool {
    if let Some(hook) = SHUTDOWN_HOOK.get() {
        hook();
        true
    } else {
        false
    }
}

/// Delete all registered directories. Called by `run_app` before every exit
/// and by the panic / signal hooks before `std::process::exit`.
pub fn run() {
    for path in DIRS.lock().iter() {
        let _ = std::fs::remove_dir_all(path);
    }
}
