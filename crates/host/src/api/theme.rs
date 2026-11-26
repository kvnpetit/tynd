//! System color-scheme + appearance preferences detection and change events.
//!
//! `dark-light` has no native subscribe on every OS; we poll every 1 s (cheap XDG/Reg/
//! CFPrefs query) and emit `theme:change` on light/dark transitions and
//! `theme:preferences-change` when any field of [`ThemeApi::preferences`]
//! changes (accent color, high contrast, reduced motion / transparency).
//! Opt-in via `os.theme.startWatch()`.
//!
//! The host exposes `current()` and `startWatch`/`stopWatch`; CSS swap
//! lives in the TS client.

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

use crate::ipc::error::IpcError;

use super::watch_refs::{WatchRefs, BACKEND_OWNER};
use crate::api::HostApi;
use crate::ipc::events;
use crate::tynd;

mod preferences;

/// Dispatch marker — watcher state lives in `WATCHER`.
#[derive(Debug, Default)]
pub(crate) struct ThemeApi;

/// Row of the host API table (`api::HOST_APIS`).
pub(crate) const API: HostApi = HostApi {
    dispatch_for_webview: Some(|method, _args, webview| dispatch_for_webview(method, webview)),
    shutdown: Some(shutdown),
    release_webview: Some(release_webview),
    ..HostApi::new("theme", |method, args| dispatch(method, &args))
};

#[tynd::handler(api = "theme")]
impl ThemeApi {
    /// Snapshot the current system color scheme. Returns `"light"`,
    /// `"dark"`, or `"unspecified"` (the latter on Linux without a
    /// portal, or on freshly-installed Windows accounts that haven't
    /// picked a scheme yet).
    pub fn current() -> String {
        current_mode()
    }

    /// Appearance snapshot: `{ mode, accentColor, highContrast,
    /// reducedMotion, reducedTransparency }`. `accentColor` is `"#rrggbb"`;
    /// every field other than `mode` is `null` where the OS does not expose
    /// it (Linux).
    pub fn preferences() -> Value {
        preferences::read().to_json(&current_mode())
    }

    /// Start the 1 s polling watcher and emit `theme:change` whenever
    /// the system mode flips, `theme:preferences-change` whenever the
    /// [`Self::preferences`] snapshot changes. Reference-counted: repeated calls return the
    /// same `{ watching: true }` payload without spawning a second thread,
    /// and each call must be balanced by a [`Self::stop_watch`].
    pub fn start_watch() -> Result<Value, String> {
        start_watch_impl(BACKEND_OWNER)
    }

    /// Release one subscription taken by [`Self::start_watch`]; the watcher
    /// stops when the last one is released. No-op when not running.
    pub fn stop_watch() {
        stop_watch_impl(BACKEND_OWNER);
    }
}

/// Renderer-originated `startWatch` / `stopWatch`, attributed to the calling
/// webview so [`release_webview`] can drop its references when it reloads or
/// closes. `None` when `method` is not a watcher method.
pub(crate) fn dispatch_for_webview(method: &str, webview: &str) -> Option<Result<Value, String>> {
    match method {
        "startWatch" => Some(start_watch_impl(webview)),
        "stopWatch" => {
            stop_watch_impl(webview);
            Some(Ok(Value::Null))
        },
        _ => None,
    }
}

/// Drop every watch reference held by `webview` (reload / close).
pub(crate) fn release_webview(webview: &str) {
    release_webview_impl(webview);
}

fn current_mode() -> String {
    match crate::platform::color_scheme::detect() {
        dark_light::Mode::Dark => "dark".into(),
        dark_light::Mode::Light => "light".into(),
        // `Unspecified` shows up on Linux without a portal or any readable
        // GTK/KDE setting, or on freshly-installed Windows accounts. Apps treat it
        // the same as "light" but get a chance to detect the lack of a
        // system-wide preference if they want.
        dark_light::Mode::Unspecified => "unspecified".into(),
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

#[derive(Default)]
struct WatchState {
    shutting_down: bool,
    watcher: Option<Watcher>,
    /// Subscriptions per owner. The watcher is a process-wide singleton, so it
    /// only stops when the last reference is gone.
    refs: WatchRefs,
}

/// `LazyLock` initializes watcher lifecycle state on first access.
static WATCHER: LazyLock<Mutex<WatchState>> = LazyLock::new(|| Mutex::new(WatchState::default()));

fn start_watch_impl(owner: &str) -> Result<Value, String> {
    let mut slot = WATCHER.lock();
    if slot.shutting_down {
        return Err(IpcError::new("theme", "startWatch", "host is shutting down").into_string());
    }
    if slot.watcher.is_some() {
        slot.refs.acquire(owner);
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
            let mut last_prefs = preferences::read();
            let mut since = Duration::ZERO;
            while r.load(Ordering::Acquire) {
                std::thread::sleep(tick);
                since += tick;
                if since < detect_every {
                    continue;
                }
                since = Duration::ZERO;
                let now = current_mode();
                let prefs = preferences::read();
                let mode_changed = now != last;
                if mode_changed {
                    last.clone_from(&now);
                    events::emit("theme:change", &json!({ "mode": now }));
                }
                if mode_changed || prefs != last_prefs {
                    last_prefs = prefs;
                    events::emit("theme:preferences-change", &prefs.to_json(&now));
                }
            }
        })
        .map_err(|e| IpcError::new("theme", "startWatch", format!("spawn: {e}")).into_string())?;

    slot.watcher = Some(Watcher {
        running,
        join: Some(join),
    });
    slot.refs.acquire(owner);
    Ok(json!({ "watching": true }))
}

fn stop_watch_impl(owner: &str) {
    let prev = {
        let mut state = WATCHER.lock();
        if !state.refs.release_one(owner) || state.refs.total() > 0 {
            return;
        }
        state.watcher.take()
    };
    if let Some(w) = prev {
        w.stop();
    }
}

fn release_webview_impl(owner: &str) {
    let prev = {
        let mut state = WATCHER.lock();
        if state.refs.release_all(owner) == 0 || state.refs.total() > 0 {
            return;
        }
        state.watcher.take()
    };
    if let Some(w) = prev {
        w.stop();
    }
}

pub(crate) fn shutdown() {
    let watcher = {
        let mut state = WATCHER.lock();
        state.shutting_down = true;
        state.watcher.take()
    };
    if let Some(watcher) = watcher {
        watcher.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn watcher_refs_are_accounted_per_webview() {
        start_watch_impl("wv-a").unwrap();
        start_watch_impl("wv-a").unwrap();
        start_watch_impl("wv-b").unwrap();
        stop_watch_impl("wv-a");
        assert!(WATCHER.lock().watcher.is_some());

        // An unbalanced stop from a webview with no refs cannot steal one.
        stop_watch_impl("wv-c");
        assert_eq!(WATCHER.lock().refs.total(), 2);

        // Reload/close of wv-a drops its remaining ref; wv-b keeps the watcher.
        release_webview("wv-a");
        assert_eq!(WATCHER.lock().refs.total(), 1);
        assert!(WATCHER.lock().watcher.is_some());

        // Releasing the last owner stops the watcher; repeats are no-ops.
        release_webview("wv-b");
        assert!(WATCHER.lock().watcher.is_none());
        release_webview("wv-b");
        stop_watch_impl("wv-b");
        assert_eq!(WATCHER.lock().refs.total(), 0);
    }
}
