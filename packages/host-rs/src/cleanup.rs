/// Global cleanup registry — paths to delete before any process::exit.
///
/// `std::process::exit` bypasses Rust's drop machinery, so `Drop`-based cleanup
/// (e.g. `tempfile::TempDir`) never runs. Instead, callers register paths here
/// and `app::run_app` calls `cleanup::run()` before every exit point.
use std::path::PathBuf;
use std::sync::Mutex;

static DIRS: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

/// Register a directory to be deleted before the process exits.
pub fn register_dir(path: PathBuf) {
    match DIRS.lock() {
        Ok(mut g) => g.push(path),
        Err(e) => crate::tynd_warn!("cleanup: mutex poisoned, temp dir will leak: {e}"),
    }
}

/// Delete all registered directories. Called by `run_app` before every exit.
pub fn run() {
    if let Ok(g) = DIRS.lock() {
        for path in g.iter() {
            let _ = std::fs::remove_dir_all(path);
        }
    }
}
