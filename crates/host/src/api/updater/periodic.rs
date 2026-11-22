use std::{
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, LazyLock,
    },
    thread::JoinHandle,
    time::Duration,
};

use parking_lot::Mutex;
use serde_json::{json, Value};

use super::{check::check_periodic, events};

/// Running periodic check, if any. Replacing / clearing the slot signals
/// the worker thread to stop and joins it.
struct PeriodicTask {
    running: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
}

impl PeriodicTask {
    fn stop(mut self) {
        self.running.store(false, Ordering::Release);
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

#[derive(Default)]
struct PeriodicState {
    shutting_down: bool,
    task: Option<PeriodicTask>,
}

static PERIODIC: LazyLock<Mutex<PeriodicState>> =
    LazyLock::new(|| Mutex::new(PeriodicState::default()));
static PERIODIC_ID: AtomicU64 = AtomicU64::new(0);

pub(super) fn start_periodic_check(args: &Value) -> Result<Value, String> {
    let mut state = PERIODIC.lock();
    if state.shutting_down {
        return Err("updater.startPeriodicCheck: host is shutting down".into());
    }
    if state.task.is_some() {
        // Already running — return the current task id unchanged.
        return Ok(json!({ "id": PERIODIC_ID.load(Ordering::Relaxed) }));
    }

    let id = PERIODIC_ID.fetch_add(1, Ordering::Relaxed) + 1;
    // 1000 ms floor keeps a typo'd interval from burning a thread at 100%.
    let interval = Duration::from_millis(
        args.get("intervalMs")
            .and_then(Value::as_u64)
            .unwrap_or(60 * 60 * 1000)
            .max(1000),
    );
    let args_owned = args.clone();
    let running = Arc::new(AtomicBool::new(true));
    let r = Arc::clone(&running);

    // 250ms tick lets `stop()` take effect in bounded time even when the
    // configured interval is long.
    let join = std::thread::Builder::new()
        .name(format!("updater-check-{id}"))
        .spawn(move || {
            let tick = Duration::from_millis(250);
            while r.load(Ordering::Acquire) {
                let result = check_periodic(&args_owned);
                if !r.load(Ordering::Acquire) {
                    break;
                }
                match result {
                    Ok(info) => events::emit(
                        "updater:check",
                        &json!({ "ok": true, "info": info, "id": id }),
                    ),
                    Err(err) => events::emit(
                        "updater:check",
                        &json!({ "ok": false, "error": err, "id": id }),
                    ),
                }
                let mut slept = Duration::ZERO;
                while slept < interval && r.load(Ordering::Acquire) {
                    let remaining = interval.saturating_sub(slept);
                    let step = tick.min(remaining);
                    std::thread::sleep(step);
                    slept += step;
                }
            }
        })
        .map_err(|e| {
            tracing::error!(
                target: "tynd::updater",
                error = %e,
                "updater.startPeriodicCheck: thread spawn failed"
            );
            format!("updater.startPeriodicCheck: thread spawn failed: {e}")
        })?;

    state.task = Some(PeriodicTask {
        running,
        join: Some(join),
    });
    Ok(json!({ "id": id }))
}

pub(super) fn stop_periodic_check() {
    let prev = PERIODIC.lock().task.take();
    if let Some(task) = prev {
        task.stop();
    }
}

pub(super) fn shutdown() {
    let task = {
        let mut state = PERIODIC.lock();
        state.shutting_down = true;
        state.task.take()
    };
    if let Some(task) = task {
        task.stop();
    }
}
