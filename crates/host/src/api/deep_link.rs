//! Deep-link OS API: system-wide custom URL scheme handler registration
//! (`myapp://...`) plus a runtime URL listener.
//!
//! # Surface
//!
//! - `register(scheme)` — make this app the OS handler for `<scheme>://…`. Per-user, persists
//!   across sessions, no admin/root required.
//! - `unregister(scheme)` — drop the OS-level association.
//! - `isRegistered(scheme)` — check whether *this* binary owns the association for `scheme`.
//! - `getStartupUrl()` — the URL the OS passed on `argv[1..]`, if any. Drained on first call
//!   (idempotent thereafter).
//! - `wasForwardedToPrimary()` — true when this process was an OS-spawned second launch that
//!   forwarded its URL to the running master via the local socket and should now exit.
//!
//! Runtime URL events are emitted as `deep-link:url { url }` on the
//! shared `crate::ipc::events` bus.
//!
//! # Per-OS notes
//!
//! - **Windows** — writes `HKCU\Software\Classes\<scheme>\…` (per-user registry, no admin).
//!   Multi-instance URL forwarding handled by the sibling `forwarding` module (master file lock +
//!   local socket).
//! - **macOS** — sets the default URL handler via `LSSetDefaultHandlerForURLScheme`. Runtime URL
//!   events come through the `NSAppleEventManager` `kInternetEventClass`/`kAEGetURL` handler that
//!   we install in [`init`].
//! - **Linux** — writes `~/.local/share/applications/<app_id>.desktop` declaring
//!   `MimeType=x-scheme-handler/<scheme>`, then shells out to `xdg-mime default …` to activate the
//!   association.
//!
//! # Init contract
//!
//! [`init`] *must* be called from `fn main` **before** any window /
//! event-loop bootstrap so a second launch carrying a deep-link URL can
//! forward to the master and `exit(0)` without ever spawning a webview.
//! The Tynd host wires this in its `run()` path.

use std::{collections::HashSet, ffi::OsString, sync::OnceLock};

use parking_lot::{Mutex, RwLock};
use serde_json::{json, Value};

use crate::api::HostApi;
use crate::ipc::events;
use crate::tynd;

#[cfg(target_os = "macos")]
mod lsregister;
#[cfg(target_os = "windows")]
mod registry;
#[cfg(target_os = "linux")]
mod xdg_desktop;

mod forwarding;

/// Process-wide state: the OS-level registration and the master lock are
/// inherently per-process.
struct State {
    /// Schemes this process called `register` on. Used by `isRegistered`
    /// as a fast-path before consulting the OS.
    schemes: RwLock<HashSet<String>>,
    /// URL captured from `argv[1..]` at init time, drained on first read.
    startup_url: RwLock<Option<String>>,
    /// Scheme of the argv startup URL. Kept after `startup_url` is drained
    /// so `relaunch_args` can still drop that URL from the forwarded argv.
    startup_scheme: RwLock<Option<String>>,
    init: Mutex<InitState>,
}

#[derive(Default)]
struct InitState {
    initialized: bool,
    shutting_down: bool,
    master_guard: Option<forwarding::MasterGuard>,
    listener: Option<crate::platform::local_listener::LocalListener>,
    forwarded_to_primary: bool,
}

static STATE: OnceLock<State> = OnceLock::new();

fn state() -> &'static State {
    STATE.get_or_init(|| State {
        schemes: RwLock::new(HashSet::new()),
        startup_url: RwLock::new(None),
        startup_scheme: RwLock::new(None),
        init: Mutex::new(InitState::default()),
    })
}

/// Initialize the deep-link subsystem.
///
/// Behaviour:
/// 1. Drain `argv[1..]` for a custom-scheme URL (skips `http`/`https`/`file`).
/// 2. Try to acquire the per-app multi-instance master lock.
///    - **Master**: spawn the local-socket listener so future second launches can post their URLs
///      to us; keep the URL for `getStartupUrl()`.
///    - **Forwarded**: set the `wasForwardedToPrimary` flag; the URL went to the master via the
///      socket — we don't keep a local copy to avoid a duplicate `deep-link:url` event.
/// 3. macOS only: install the Apple Event handler so `GURL` events arriving while the app is
///    already running surface as `deep-link:url`.
///
/// Idempotent — a second call after the master is already established
/// is a no-op.
pub(crate) fn init() -> Result<(), String> {
    let st = state();
    let mut init = st.init.lock();
    if init.shutting_down {
        return Err("deepLink.init: host is shutting down".into());
    }
    if init.initialized {
        return Ok(());
    }

    // argv-based cold-start URL. Skip argv[0] (exe path).
    let found_url = std::env::args()
        .skip(1)
        .find_map(|arg| parse_custom_scheme_url(&arg));
    if let Some(scheme) = found_url.as_deref().and_then(custom_scheme_of) {
        *st.startup_scheme.write() = Some(scheme);
    }

    let app_id = forwarding_app_id();
    match forwarding::acquire_or_forward(&app_id, found_url.as_deref()) {
        Ok(forwarding::AcquireOutcome::Master(guard)) => {
            if let Err(error) = become_master(&mut init, &app_id, guard) {
                crate::tynd_warn!("deep-link forwarding listener disabled: {error}");
            }
            if let Some(url) = found_url {
                *st.startup_url.write() = Some(url);
            }
        },
        Ok(forwarding::AcquireOutcome::Forwarded) => {
            init.forwarded_to_primary = true;
        },
        Err(e) => {
            // Filesystem-level failure (no cache dir, lock-file open
            // failed). Only the cold-start URL is delivered; the rest of
            // the API surface still works.
            crate::tynd_warn!("deep-link forwarding disabled: {e}");
            if let Some(url) = found_url {
                *st.startup_url.write() = Some(url);
            }
        },
    }

    // macOS Apple Event handler — runtime `GURL` events while the app
    // is already running. Forwards to `deep-link:url` on the same
    // event bus the master listener uses.
    #[cfg(target_os = "macos")]
    lsregister::install_apple_event_handler();

    init.initialized = true;
    Ok(())
}

/// `true` when this process was an OS-spawned second launch that
/// forwarded its startup URL to the master and should exit. Cheap to
/// poll — does not require [`init`] to have completed (defaults to
/// `false` if `init` was never called).
pub(crate) fn was_forwarded_to_primary() -> bool {
    STATE
        .get()
        .is_some_and(|state| state.init.lock().forwarded_to_primary)
}

/// Keep the master lock only together with a running listener. Without
/// one, later launches would fail the lock and forward into a socket that
/// cannot accept their URL, so on a listener error `guard` is dropped.
fn become_master(
    init: &mut InitState,
    app_id: &str,
    guard: forwarding::MasterGuard,
) -> Result<(), String> {
    let listener = forwarding::spawn_listener(app_id)?;
    init.master_guard = Some(guard);
    init.listener = Some(listener);
    Ok(())
}

/// Forwarding listener and master lock, as taken out of [`InitState`].
type MasterParts = (
    Option<crate::platform::local_listener::LocalListener>,
    Option<forwarding::MasterGuard>,
);

/// Take the listener and master lock out of `init`; the caller stops them
/// with [`stop_master`] once the state lock is released.
fn take_master(init: &mut InitState) -> MasterParts {
    (init.listener.take(), init.master_guard.take())
}

/// Join the listener, then release the master lock. Returns `true` when a
/// lock was held.
fn stop_master((listener, master_guard): MasterParts) -> bool {
    if let Some(listener) = listener {
        listener.shutdown();
    }
    master_guard.is_some()
}

pub(crate) fn shutdown() {
    let Some(state) = STATE.get() else {
        return;
    };
    let master = {
        let mut init = state.init.lock();
        init.shutting_down = true;
        init.initialized = true;
        take_master(&mut init)
    };
    stop_master(master);
}

/// Release the deep-link master lock and listener so a relaunched copy of
/// this binary becomes the master instead of forwarding to this dying
/// process (and exiting). Returns `true` when a lock was released.
pub(crate) fn release_for_relaunch() -> bool {
    let Some(state) = STATE.get() else {
        return false;
    };
    let master = take_master(&mut state.init.lock());
    stop_master(master)
}

/// Re-take the master lock released by [`release_for_relaunch`] after the
/// relaunch failed, so this process keeps receiving forwarded URLs.
pub(crate) fn restore_after_failed_relaunch() {
    let Some(state) = STATE.get() else {
        return;
    };
    let mut init = state.init.lock();
    if init.shutting_down || init.master_guard.is_some() {
        return;
    }
    let app_id = forwarding_app_id();
    match forwarding::acquire_or_forward(&app_id, None) {
        Ok(forwarding::AcquireOutcome::Master(guard)) => {
            if let Err(error) = become_master(&mut init, &app_id, guard) {
                crate::tynd_warn!("deep-link listener not restored: {error}");
            }
        },
        Ok(forwarding::AcquireOutcome::Forwarded) => crate::tynd_warn!(
            "deep-link master lock was taken by another process during a failed relaunch"
        ),
        Err(error) => crate::tynd_warn!("deep-link master lock not re-acquired: {error}"),
    }
}

/// Dispatch marker — deep-link state lives in `STATE`.
#[derive(Debug, Default)]
pub(crate) struct DeepLinkApi;

/// Row of the host API table (`api::HOST_APIS`).
pub(crate) const API: HostApi = HostApi {
    shutdown: Some(shutdown),
    ..HostApi::new("deepLink", |method, args| dispatch(method, &args))
};

#[tynd::handler(api = "deepLink")]
impl DeepLinkApi {
    /// Register `scheme` (e.g. `"tynd-app"`) with the OS so URLs
    /// matching `<scheme>://...` route back to this binary.
    pub fn register(scheme: String) -> Result<(), String> {
        register_impl(&scheme)
    }

    /// Unregister `scheme` from the OS.
    pub fn unregister(scheme: String) -> Result<(), String> {
        unregister_impl(&scheme)
    }

    /// True when `scheme` is currently registered for the running
    /// binary.
    pub fn is_registered(scheme: String) -> Result<bool, String> {
        is_registered_impl(&scheme)
    }

    /// One-shot startup URL — the deep link the host was launched
    /// with, if any. Returns `null` after the first read so a
    /// re-render doesn't see a stale value.
    pub fn get_startup_url() -> Value {
        get_startup_url()
    }

    /// True when the current process forwarded its launch URL to a
    /// primary instance (and therefore should exit silently).
    pub fn was_forwarded_to_primary() -> bool {
        was_forwarded_to_primary()
    }
}

fn register_impl(scheme: &str) -> Result<(), String> {
    validate_scheme(scheme)?;
    let exe = current_exe_string()?;

    #[cfg(target_os = "windows")]
    registry::register(scheme, &exe).map_err(|e| format!("deepLink.register: {e}"))?;

    #[cfg(target_os = "macos")]
    lsregister::register(scheme).map_err(|e| format!("deepLink.register: {e}"))?;

    #[cfg(target_os = "linux")]
    xdg_desktop::register(scheme, &exe).map_err(|e| format!("deepLink.register: {e}"))?;

    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        let _ = exe;
        // Refuse the registration outright — silently inserting into
        // `state().schemes` would make `is_registered` lie about an
        // OS-level association that was never established.
        return Err("deepLink.register: unsupported platform".to_string());
    }

    #[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
    {
        state().schemes.write().insert(scheme.to_string());
        Ok(())
    }
}

fn unregister_impl(scheme: &str) -> Result<(), String> {
    validate_scheme(scheme)?;

    #[cfg(target_os = "windows")]
    registry::unregister(scheme).map_err(|e| format!("deepLink.unregister: {e}"))?;

    #[cfg(target_os = "macos")]
    lsregister::unregister(scheme).map_err(|e| format!("deepLink.unregister: {e}"))?;

    #[cfg(target_os = "linux")]
    xdg_desktop::unregister(scheme).map_err(|e| format!("deepLink.unregister: {e}"))?;

    state().schemes.write().remove(scheme);
    Ok(())
}

fn is_registered_impl(scheme: &str) -> Result<bool, String> {
    validate_scheme(scheme)?;

    // The OS association is the truth: another app can take the scheme over
    // during the session. The in-memory set only answers when the OS query
    // itself fails.
    #[cfg(target_os = "windows")]
    let os_owned = registry::is_registered(scheme, &current_exe_string().unwrap_or_default()).ok();
    #[cfg(target_os = "macos")]
    let os_owned = lsregister::is_registered(scheme).ok();
    #[cfg(target_os = "linux")]
    let os_owned = xdg_desktop::is_registered(scheme).ok();
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    let os_owned = Some(false);

    Ok(os_owned.unwrap_or_else(|| state().schemes.read().contains(scheme)))
}

fn get_startup_url() -> Value {
    match state().startup_url.write().take() {
        Some(s) => Value::String(s),
        None => Value::Null,
    }
}

/// Reject anything that isn't an RFC-3986 scheme. Bounded length so
/// callers can't pass pathological strings that would balloon the
/// registry / .desktop file. Leading char must be alpha — common bug
/// class on Windows is a scheme starting with a digit, which the
/// registry tolerates but browsers refuse to launch.
fn validate_scheme(scheme: &str) -> Result<(), String> {
    if scheme.is_empty() || scheme.len() > 64 {
        return Err(format!("deepLink: scheme '{scheme}' must be 1..=64 chars"));
    }
    let mut chars = scheme.chars();
    let first = chars
        .next()
        .ok_or_else(|| "deepLink: scheme is empty".to_string())?;
    if !first.is_ascii_alphabetic() {
        return Err(format!(
            "deepLink: scheme '{scheme}' must start with an ASCII letter"
        ));
    }
    for c in chars {
        if !(c.is_ascii_alphanumeric() || c == '+' || c == '-' || c == '.') {
            return Err(format!(
                "deepLink: scheme '{scheme}' has invalid char '{c}'"
            ));
        }
    }
    Ok(())
}

/// Walk a single argv element looking for a custom-scheme URL. Filters
/// out `http`, `https`, and `file` so we don't capture drag-and-dropped
/// webpage shortcuts or local file launches.
fn parse_custom_scheme_url(arg: &str) -> Option<String> {
    custom_scheme_of(arg).map(|_| arg.to_string())
}

/// Lowercase scheme of `arg` when it is a custom-scheme URL that
/// `parse_custom_scheme_url` would accept.
fn custom_scheme_of(arg: &str) -> Option<String> {
    let url = url::Url::parse(arg).ok()?;
    let scheme = url.scheme();
    if matches!(
        scheme,
        "" | "http"
            | "https"
            | "file"
            | "javascript"
            | "data"
            | "vbscript"
            | "about"
            | "blob"
            | "view-source"
            | "chrome"
            | "chrome-extension"
            | "ipc"
            | "tynd"
            | "tynd-bin"
    ) {
        return None;
    }
    Some(scheme.to_ascii_lowercase())
}

/// Arguments to forward to a relaunched process (`argv[1..]`). Deep-link
/// URLs whose scheme this process registered, or whose scheme delivered
/// its startup URL, are dropped: the new process would otherwise pick the
/// same URL up as its startup URL and handle it a second time.
pub(crate) fn relaunch_args() -> Vec<OsString> {
    let st = state();
    let mut schemes: HashSet<String> = st
        .schemes
        .read()
        .iter()
        .map(|scheme| scheme.to_ascii_lowercase())
        .collect();
    if let Some(scheme) = st.startup_scheme.read().clone() {
        schemes.insert(scheme);
    }
    filter_relaunch_args(std::env::args_os().skip(1), &schemes)
}

fn filter_relaunch_args(
    args: impl IntoIterator<Item = OsString>,
    schemes: &HashSet<String>,
) -> Vec<OsString> {
    args.into_iter()
        .filter(|arg| {
            !matches!(
                arg.to_str().and_then(custom_scheme_of),
                Some(scheme) if schemes.contains(&scheme)
            )
        })
        .collect()
}

/// Per-app identifier used to scope the multi-instance lock + socket. The
/// first non-empty of: the macOS `__CFBundleIdentifier` env var (set by
/// Cocoa), the `TYND_DEEPLINK_APP_ID` env var, then the executable basename
/// so dev builds running under `cargo run` still get an identifier.
fn forwarding_app_id() -> String {
    ["__CFBundleIdentifier", "TYND_DEEPLINK_APP_ID"]
        .into_iter()
        .find_map(|name| std::env::var(name).ok().filter(|id| !id.is_empty()))
        .or_else(|| {
            std::env::current_exe()
                .ok()
                .and_then(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
        })
        .unwrap_or_else(|| "tynd-app".into())
}

/// Best-effort path to the current executable as a UTF-8 string.
fn current_exe_string() -> Result<String, String> {
    let p = std::env::current_exe().map_err(|e| format!("deepLink: current_exe: {e}"))?;
    p.to_str()
        .map(str::to_string)
        .ok_or_else(|| "deepLink: current_exe path is not valid UTF-8".to_string())
}

/// Emit a `deep-link:url` event onto the shared OS event bus. Used by
/// the macOS Apple Event handler and the master-side socket listener.
pub(crate) fn emit_url(url: &str) {
    events::emit("deep-link:url", &json!({ "url": url }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relaunch_args_drop_registered_deep_links_only() {
        let schemes: HashSet<String> = ["myapp".to_string()].into_iter().collect();
        let args = [
            "--flag",
            "myapp://do?x=1",
            "MyApp://other",
            "otherapp://keep",
            "https://example.com",
            r"C:\docs\file.txt",
        ]
        .map(OsString::from);
        let kept = filter_relaunch_args(args, &schemes);
        assert_eq!(
            kept,
            [
                "--flag",
                "otherapp://keep",
                "https://example.com",
                r"C:\docs\file.txt"
            ]
            .map(OsString::from)
        );
    }

    #[test]
    fn release_for_relaunch_frees_the_master_lock() {
        let app_id = format!("tynd-test-relaunch-{}", std::process::id());
        let forwarding::AcquireOutcome::Master(guard) =
            forwarding::acquire_or_forward(&app_id, None).expect("first acquire")
        else {
            panic!("first acquire must become master");
        };
        state().init.lock().master_guard = Some(guard);

        // While held, a second launch would forward and exit.
        assert!(matches!(
            forwarding::acquire_or_forward(&app_id, None),
            Ok(forwarding::AcquireOutcome::Forwarded)
        ));

        assert!(release_for_relaunch());
        assert!(!release_for_relaunch());
        assert!(matches!(
            forwarding::acquire_or_forward(&app_id, None),
            Ok(forwarding::AcquireOutcome::Master(_))
        ));
    }
}
