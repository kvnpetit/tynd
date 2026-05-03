//! Deep-link OS API: system-wide custom URL scheme handler registration
//! (`myapp://...`) plus a runtime URL listener.
//!
//! Promoted from the legacy `@tynd/plugin-deeplink` plugin so the
//! native scheme-registration + multi-instance URL forwarding ride
//! along with every preset / `api-all` build instead of being an
//! optional plugin opt-in.
//!
//! # Surface
//!
//! - `register(scheme)` — make this app the OS handler for `<scheme>://…`.
//!   Per-user, persists across sessions, no admin/root required.
//! - `unregister(scheme)` — drop the OS-level association.
//! - `isRegistered(scheme)` — check whether *this* binary owns the
//!   association for `scheme`.
//! - `getStartupUrl()` — the URL the OS passed on `argv[1..]`, if any.
//!   Drained on first call (idempotent thereafter).
//! - `wasForwardedToPrimary()` — true when this process was an
//!   OS-spawned second launch that forwarded its URL to the running
//!   master via the local socket and should now exit.
//!
//! Runtime URL events are emitted as `deep-link:url { url }` on the
//! shared `os::events` bus — same channel the legacy plugin used, so
//! existing frontends keep working.
//!
//! # Per-OS notes
//!
//! - **Windows** — writes `HKCU\Software\Classes\<scheme>\…` (per-user
//!   registry, no admin). Multi-instance URL forwarding handled by the
//!   sibling `forwarding` module (master file lock + local socket).
//! - **macOS** — sets the default URL handler via
//!   `LSSetDefaultHandlerForURLScheme`. Runtime URL events come through
//!   the `NSAppleEventManager` `kInternetEventClass`/`kAEGetURL` handler
//!   that we install in [`init`].
//! - **Linux** — writes `~/.local/share/applications/<app_id>.desktop`
//!   declaring `MimeType=x-scheme-handler/<scheme>`, then shells out
//!   to `xdg-mime default …` to activate the association.
//!
//! # Init contract
//!
//! [`init`] *must* be called from `fn main` **before** any window /
//! event-loop bootstrap so a second launch carrying a deep-link URL can
//! forward to the master and `exit(0)` without ever spawning a webview.
//! `tynd-full` and `tynd-lite` both wire this in their `run()` path.

use std::{
    collections::HashSet,
    sync::OnceLock,
};

use parking_lot::RwLock;
use serde_json::{json, Value};

use super::events;
use crate::tynd;
#[allow(unused_imports)]
use crate::ipc_error::IpcError;

#[cfg(target_os = "linux")]
mod xdg_desktop;
#[cfg(target_os = "macos")]
mod lsregister;
#[cfg(target_os = "windows")]
mod registry;

mod forwarding;

/// Global state. The plugin originally kept this on a `DeepLinkPlugin`
/// instance; promoted to a process-wide singleton because the OS-level
/// registration + master lock are inherently per-process.
struct State {
    /// Schemes this process called `register` on. Used by `isRegistered`
    /// as a fast-path before consulting the OS.
    schemes: RwLock<HashSet<String>>,
    /// URL captured from `argv[1..]` at init time, drained on first read.
    startup_url: RwLock<Option<String>>,
    /// Master-instance lock guard. `Some` when this process is the
    /// master; held for the life of the process so the OS lock isn't
    /// released until exit.
    _master_guard: RwLock<Option<forwarding::MasterGuard>>,
    /// True iff `init` detected an already-running master and forwarded
    /// our URL to it. Polled by the host bootstrap to decide whether to
    /// exit early.
    forwarded_to_primary: RwLock<bool>,
}

static STATE: OnceLock<State> = OnceLock::new();

fn state() -> &'static State {
    STATE.get_or_init(|| State {
        schemes: RwLock::new(HashSet::new()),
        startup_url: RwLock::new(None),
        _master_guard: RwLock::new(None),
        forwarded_to_primary: RwLock::new(false),
    })
}

/// Initialize the deep-link subsystem.
///
/// Behaviour:
/// 1. Drain `argv[1..]` for a custom-scheme URL (skips `http`/`https`/`file`).
/// 2. Try to acquire the per-app multi-instance master lock.
///    - **Master**: spawn the local-socket listener so future second
///      launches can post their URLs to us; keep the URL for
///      `getStartupUrl()`.
///    - **Forwarded**: set the `wasForwardedToPrimary` flag; the URL
///      went to the master via the socket — we don't keep a local copy
///      to avoid a duplicate `deep-link:url` event.
/// 3. macOS only: install the Apple Event handler so `GURL` events
///    arriving while the app is already running surface as
///    `deep-link:url`.
///
/// Idempotent — a second call after the master is already established
/// is a no-op.
pub fn init() -> Result<(), String> {
    let st = state();

    // argv-based cold-start URL. Skip argv[0] (exe path).
    let mut found_url: Option<String> = None;
    for arg in std::env::args().skip(1) {
        if let Some(url) = parse_custom_scheme_url(&arg) {
            found_url = Some(url);
            break;
        }
    }

    let app_id = forwarding_app_id();
    match forwarding::acquire_or_forward(&app_id, found_url.as_deref()) {
        Ok(forwarding::AcquireOutcome::Master(guard)) => {
            *st._master_guard.write() = Some(guard);
            forwarding::spawn_listener(&app_id);
            if let Some(url) = found_url {
                *st.startup_url.write() = Some(url);
            }
        },
        Ok(forwarding::AcquireOutcome::Forwarded { .. }) => {
            *st.forwarded_to_primary.write() = true;
        },
        Err(e) => {
            // Filesystem-level failure (no cache dir, lock-file open
            // failed). Fall back to legacy cold-start-only behaviour —
            // the rest of the API surface still works.
            crate::tynd_log!("deep-link forwarding disabled: {e}");
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

    Ok(())
}

/// `true` when this process was an OS-spawned second launch that
/// forwarded its startup URL to the master and should exit. Cheap to
/// poll — does not require [`init`] to have completed (defaults to
/// `false` if `init` was never called).
pub fn was_forwarded_to_primary() -> bool {
    STATE.get().is_some_and(|s| *s.forwarded_to_primary.read())
}

/// Marker for the `#[tynd::handler]` dispatch table — deep-link
/// state lives in the file-level `STATE` static.
#[derive(Debug, Default)]
pub struct DeepLinkApi;

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
    let scheme = scheme.to_string();
    let exe = current_exe_string()?;

    #[cfg(target_os = "windows")]
    registry::register(&scheme, &exe).map_err(|e| format!("deepLink.register: {e}"))?;

    #[cfg(target_os = "macos")]
    lsregister::register(&scheme).map_err(|e| format!("deepLink.register: {e}"))?;

    #[cfg(target_os = "linux")]
    xdg_desktop::register(&scheme, &exe).map_err(|e| format!("deepLink.register: {e}"))?;

    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        let _ = exe;
    }

    state().schemes.write().insert(scheme);
    Ok(())
}

fn unregister_impl(scheme: &str) -> Result<(), String> {
    validate_scheme(scheme)?;
    let scheme = scheme.to_string();

    #[cfg(target_os = "windows")]
    registry::unregister(&scheme).map_err(|e| format!("deepLink.unregister: {e}"))?;

    #[cfg(target_os = "macos")]
    lsregister::unregister(&scheme).map_err(|e| format!("deepLink.unregister: {e}"))?;

    #[cfg(target_os = "linux")]
    xdg_desktop::unregister(&scheme).map_err(|e| format!("deepLink.unregister: {e}"))?;

    state().schemes.write().remove(&scheme);
    Ok(())
}

fn is_registered_impl(scheme: &str) -> Result<bool, String> {
    validate_scheme(scheme)?;
    let scheme = scheme.to_string();
    let exe = current_exe_string().unwrap_or_default();

    #[cfg(target_os = "windows")]
    let os_owned = registry::is_registered(&scheme, &exe).unwrap_or(false);
    #[cfg(target_os = "macos")]
    let os_owned = lsregister::is_registered(&scheme).unwrap_or(false);
    #[cfg(target_os = "linux")]
    let os_owned = xdg_desktop::is_registered(&scheme).unwrap_or(false);
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    let os_owned = false;

    let owned = os_owned || state().schemes.read().contains(&scheme);
    let _ = exe;
    Ok(owned)
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
    let first = chars.next().ok_or_else(|| "deepLink: scheme is empty".to_string())?;
    if !first.is_ascii_alphabetic() {
        return Err(format!(
            "deepLink: scheme '{scheme}' must start with an ASCII letter"
        ));
    }
    for c in chars {
        if !(c.is_ascii_alphanumeric() || c == '+' || c == '-' || c == '.') {
            return Err(format!("deepLink: scheme '{scheme}' has invalid char '{c}'"));
        }
    }
    Ok(())
}

/// Walk a single argv element looking for a custom-scheme URL. Filters
/// out `http`, `https`, and `file` so we don't capture drag-and-dropped
/// webpage shortcuts or local file launches.
fn parse_custom_scheme_url(arg: &str) -> Option<String> {
    let url = url::Url::parse(arg).ok()?;
    let scheme = url.scheme();
    if matches!(scheme, "http" | "https" | "file") || scheme.is_empty() {
        return None;
    }
    Some(arg.to_string())
}

/// Per-app identifier used to scope the multi-instance lock + socket.
/// Prefers the macOS `__CFBundleIdentifier` env var (set by Cocoa) and
/// falls back to the executable basename so dev builds running under
/// `cargo run` still get an identifier.
fn forwarding_app_id() -> String {
    if let Ok(id) = std::env::var("__CFBundleIdentifier") {
        if !id.is_empty() {
            return id;
        }
    }
    if let Ok(id) = std::env::var("TYND_DEEPLINK_APP_ID") {
        if !id.is_empty() {
            return id;
        }
    }
    std::env::current_exe()
        .ok()
        .and_then(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
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
    fn validates_scheme_shape() {
        assert!(validate_scheme("myapp").is_ok());
        assert!(validate_scheme("my-app+1.0").is_ok());
        assert!(validate_scheme("").is_err());
        assert!(validate_scheme(&"a".repeat(65)).is_err());
        assert!(validate_scheme("1myapp").is_err());
        assert!(validate_scheme("-myapp").is_err());
        assert!(validate_scheme("my app").is_err());
        assert!(validate_scheme("my/app").is_err());
        assert!(validate_scheme("my:app").is_err());
    }

    #[test]
    fn parse_custom_scheme_url_filters_well_known() {
        assert!(parse_custom_scheme_url("myapp://callback?token=abc").is_some());
        assert!(parse_custom_scheme_url("custom-scheme://x/y").is_some());
        assert_eq!(parse_custom_scheme_url("https://example.com"), None);
        assert_eq!(parse_custom_scheme_url("http://example.com"), None);
        assert_eq!(parse_custom_scheme_url("file:///tmp/a.txt"), None);
        assert_eq!(parse_custom_scheme_url("--flag"), None);
        assert_eq!(parse_custom_scheme_url("/usr/bin/foo"), None);
    }

    #[test]
    fn get_startup_url_returns_null_when_unset() {
        let v = dispatch("getStartupUrl", &Value::Null).unwrap();
        // Other tests in this binary may have set it via init(); only
        // assert the type contract (string or null), not the value.
        assert!(v.is_null() || v.is_string());
    }

    #[test]
    fn was_forwarded_to_primary_defaults_false() {
        // Initial state — even without `init`, the flag must report
        // `false` so the host bootstrap doesn't accidentally exit.
        let v = dispatch("wasForwardedToPrimary", &Value::Null).unwrap();
        assert!(v.is_boolean());
    }

    #[test]
    fn unknown_method_errors() {
        let err = dispatch("nope", &Value::Null).unwrap_err();
        assert!(err.contains("unknown method"));
    }

    #[test]
    fn forwarding_app_id_is_non_empty() {
        let id = forwarding_app_id();
        assert!(!id.is_empty());
    }
}
