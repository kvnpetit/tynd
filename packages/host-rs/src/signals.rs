//! Production-grade signal handling — Ctrl+C / SIGTERM / Windows logoff.
//!
//! On the first signal we ask the running tao event loop to exit cleanly via
//! the shutdown hook registered in `cleanup::set_shutdown_hook`. The event
//! loop runs `cleanup::run()` and `std::process::exit(0)` at its next
//! convenient point (before the next frame, drag, IPC dispatch, etc.).
//!
//! On a *second* signal arriving within `HARD_EXIT_WINDOW_MS` of the first
//! we skip the cooperative path and call `std::process::exit(130)` directly
//! (130 = 128 + SIGINT, the conventional shell exit code for Ctrl+C). This
//! mirrors how production daemons behave: the operator can always escalate
//! by hitting Ctrl+C twice if the app's shutdown handler is wedged.
//!
//! Implementation notes:
//! - The `ctrlc` crate spawns one OS-owned thread on Windows (`SetConsoleCtrlHandler` runs there)
//!   and uses `sigaction` on Unix. We avoid heap allocations in the hot path: the closure captures
//!   only one `OnceLock`-backed `Instant` and a global `AtomicBool`.
//! - `set_handler` errors only when called twice or when the platform refuses to install the
//!   handler — both are non-fatal here, so we log and move on rather than aborting startup.

use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        OnceLock,
    },
    time::{Duration, Instant},
};

use parking_lot::Mutex;

/// How long after the first signal a second one is treated as "force quit"
/// instead of a duplicate. Long enough to cover slow event-loop wake-ups
/// (e.g. modal native dialogs) without making the user wait if they
/// genuinely meant to escalate.
const HARD_EXIT_WINDOW_MS: u64 = 2_000;

/// Tracks whether we've already received a graceful-shutdown signal.
/// Read on every signal hit; written exactly once.
static FIRST_SIGNAL_AT: OnceLock<Mutex<Option<Instant>>> = OnceLock::new();
/// Idempotency flag for `init_signals()`. Multiple presets calling it (or
/// integration tests) must not re-register the OS handler.
static INSTALLED: AtomicBool = AtomicBool::new(false);

/// Install the Ctrl+C / SIGTERM handler. Idempotent. Safe to call from
/// `init_tracing()` even if no event loop will ever start (the hook just
/// runs `cleanup::run()` and exits in that case).
///
/// Hosts (`tynd-full` / `tynd-lite`) call this once at startup, right
/// after `init_tracing()`.
pub fn init() {
    if INSTALLED.swap(true, Ordering::SeqCst) {
        return;
    }

    let _ = FIRST_SIGNAL_AT.set(Mutex::new(None));

    let result = ctrlc::set_handler(|| {
        on_signal();
    });

    if let Err(e) = result {
        // Most common cause: another library already installed a handler
        // (e.g. a test harness, or a second `init()` racing through before
        // the `INSTALLED` swap). Log and continue — the app still works,
        // we just lose the graceful path.
        tracing::warn!(
            target: "tynd::signals",
            error = %e,
            "failed to install Ctrl+C / SIGTERM handler — falling back to default OS behaviour"
        );
    } else {
        tracing::debug!(
            target: "tynd::signals",
            "Ctrl+C / SIGTERM handler installed (graceful first hit, hard exit on second within {HARD_EXIT_WINDOW_MS}ms)"
        );
    }
}

/// Body of the signal handler. Kept allocation-light: one mutex lock, one
/// `Instant::now()`, and a logging call. Heavy work (`cleanup::run()`,
/// `std::process::exit`) happens either inline on the hard-exit branch or
/// later inside the event loop on the soft branch.
fn on_signal() {
    let Some(cell) = FIRST_SIGNAL_AT.get() else {
        // `init()` wasn't called — defensive path. Just exit immediately;
        // the app never registered a graceful-shutdown hook anyway.
        crate::cleanup::run();
        std::process::exit(130);
    };

    let mut slot = cell.lock();
    let now = Instant::now();

    match *slot {
        Some(prev) if now.duration_since(prev) <= Duration::from_millis(HARD_EXIT_WINDOW_MS) => {
            // Second signal within the window — operator escalation.
            // Drop the lock before exiting so any concurrent first-signal
            // path doesn't deadlock during cleanup.
            drop(slot);
            tracing::warn!(
                target: "tynd::signals",
                "second shutdown signal received within {HARD_EXIT_WINDOW_MS}ms — forcing exit (code 130)"
            );
            crate::cleanup::run();
            std::process::exit(130);
        },
        _ => {
            *slot = Some(now);
            drop(slot);
            tracing::info!(
                target: "tynd::signals",
                "shutdown signal received, exiting cleanly (send signal again within {HARD_EXIT_WINDOW_MS}ms to force)"
            );
            // Ask the event loop to wind down. If no hook is registered
            // (e.g. signal arrived before `run_app` had a chance to set
            // one), fall back to a direct exit so the user isn't left
            // staring at a wedged process.
            if !crate::cleanup::trigger_shutdown() {
                crate::cleanup::run();
                std::process::exit(130);
            }
        },
    }
}
