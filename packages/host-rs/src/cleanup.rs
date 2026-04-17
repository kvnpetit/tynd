/// Global cleanup registry — paths to delete before any process::exit.
///
/// `std::process::exit` bypasses Rust's drop machinery, so `Drop`-based cleanup
/// (e.g. `tempfile::TempDir`) never runs. Instead, callers register paths here
/// and `app::run_app` calls `cleanup::run()` before every exit point.
use parking_lot::Mutex;
use std::path::PathBuf;

static DIRS: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

/// Register a directory to be deleted before the process exits.
pub fn register_dir(path: PathBuf) {
    DIRS.lock().push(path);
}

/// Delete all registered directories. Called by `run_app` before every exit.
pub fn run() {
    for path in DIRS.lock().iter() {
        let _ = std::fs::remove_dir_all(path);
    }
}
