//! Ctrl+C / SIGTERM / Windows logoff handling.
//!
//! First signal: ask the event loop to exit via `cleanup::trigger_shutdown`.
//! Second signal within `HARD_EXIT_WINDOW_MS`: hard `exit(130)`. Mirrors how
//! production daemons let operators escalate when the cooperative path wedges.

use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        LazyLock,
    },
    time::{Duration, Instant},
};

use parking_lot::Mutex;

/// Long enough to cover slow event-loop wake-ups (modal native dialogs)
/// without making the user wait if they meant to escalate.
const HARD_EXIT_WINDOW_MS: u64 = 2_000;

static FIRST_SIGNAL_AT: LazyLock<Mutex<Option<Instant>>> = LazyLock::new(|| Mutex::new(None));
static INSTALLED: AtomicBool = AtomicBool::new(false);

/// Install the Ctrl+C / SIGTERM handler. Idempotent.
pub(crate) fn init() {
    if INSTALLED.swap(true, Ordering::AcqRel) {
        return;
    }

    let result = ctrlc::set_handler(|| {
        on_signal();
    });

    if let Err(e) = result {
        // Usually a test harness already installed a handler. Log + continue —
        // we lose the graceful path but the app still works.
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

/// Signal-handler body. Allocation-light: one mutex + `Instant::now()` + log.
fn on_signal() {
    let mut slot = FIRST_SIGNAL_AT.lock();
    let now = Instant::now();

    match *slot {
        Some(prev) if now.duration_since(prev) <= Duration::from_millis(HARD_EXIT_WINDOW_MS) => {
            // Drop the lock before exiting. The first signal already started
            // graceful cleanup; retrying it here could block behind a wedged
            // plugin and defeat the explicit hard-exit escape hatch.
            drop(slot);
            tracing::warn!(
                target: "tynd::signals",
                "second shutdown signal received within {HARD_EXIT_WINDOW_MS}ms — forcing exit (code 130)"
            );
            std::process::exit(130);
        },
        _ => {
            *slot = Some(now);
            drop(slot);
            tracing::info!(
                target: "tynd::signals",
                "shutdown signal received, exiting cleanly (send signal again within {HARD_EXIT_WINDOW_MS}ms to force)"
            );
            // Fall back to a hard exit when no shutdown hook is registered
            // (signal arrived before `run_app` could set one).
            if !crate::cleanup::trigger_shutdown() {
                crate::cleanup::run();
                std::process::exit(130);
            }
        },
    }
}
