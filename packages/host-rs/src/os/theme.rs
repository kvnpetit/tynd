//! System color-scheme detection + change events.
//!
//! `dark-light` 2.0 only exposes a synchronous `detect()` snapshot — there's
//! no native `subscribe` API. We poll on a 1 s interval (cheap: each call
//! is a single XDG/Reg/CFPrefs query) and emit `theme:change` whenever the
//! mode flips. The poller is opt-in via `os.theme.startWatch()` so we
//! don't burn a thread when no listener is installed.
//!
//! The light/dark *application* logic (CSS variable swap, html class,
//! palette persistence) lives in the TS client (`core/src/client/theme.ts`)
//! since it touches DOM APIs the host doesn't have. The host owns:
//! - `current()` → "light" | "dark" | "unspecified"
//! - `startWatch()` / `stopWatch()` → emits `theme:change` on transition.

use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, LazyLock,
    },
    thread::JoinHandle,
    time::Duration,
};

use parking_lot::Mutex;
use serde_json::{json, Value};

use super::events;

pub fn dispatch(method: &str, _args: &Value) -> Result<Value, String> {
    match method {
        "current" => Ok(Value::String(current_mode())),
        "startWatch" => start_watch(),
        "stopWatch" => {
            stop_watch();
            Ok(Value::Null)
        },
        _ => Err(format!("theme.{method}: unknown method")),
    }
}

fn current_mode() -> String {
    match dark_light::detect() {
        Ok(dark_light::Mode::Dark) => "dark".into(),
        Ok(dark_light::Mode::Light) => "light".into(),
        // `Unspecified` shows up on Linux without a portal, or on
        // freshly-installed Windows accounts. Apps treat it the same as
        // "light" but get a chance to detect the lack of a system-wide
        // preference if they want.
        _ => "unspecified".into(),
    }
}

struct Watcher {
    running: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
}

impl Watcher {
    fn stop(mut self) {
        self.running.store(false, Ordering::Release);
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

/// `LazyLock` initializes the inner `Mutex<Option<Watcher>>` to `None`
/// on first access — replaces a `watcher_slot()` accessor wrapping
/// `OnceLock::get_or_init`.
static WATCHER: LazyLock<Mutex<Option<Watcher>>> = LazyLock::new(|| Mutex::new(None));

fn start_watch() -> Result<Value, String> {
    let mut slot = WATCHER.lock();
    if slot.is_some() {
        return Ok(json!({ "watching": true }));
    }
    let running = Arc::new(AtomicBool::new(true));
    let r = Arc::clone(&running);

    let join = std::thread::Builder::new()
        .name("theme-watch".into())
        .spawn(move || {
            // 250ms tick lets stop() take effect quickly while the
            // detection itself only runs once a second.
            let detect_every = Duration::from_secs(1);
            let tick = Duration::from_millis(250);
            let mut last = current_mode();
            let mut since = Duration::ZERO;
            while r.load(Ordering::Acquire) {
                std::thread::sleep(tick);
                since += tick;
                if since < detect_every {
                    continue;
                }
                since = Duration::ZERO;
                let now = current_mode();
                if now != last {
                    last.clone_from(&now);
                    events::emit("theme:change", &json!({ "mode": now }));
                }
            }
        })
        .map_err(|e| format!("theme.startWatch: spawn: {e}"))?;

    *slot = Some(Watcher {
        running,
        join: Some(join),
    });
    Ok(json!({ "watching": true }))
}

fn stop_watch() {
    let prev = WATCHER.lock().take();
    if let Some(w) = prev {
        w.stop();
    }
}
