//! Process-wide callback registry for breaking host crate cycles.
//!
//! Background:
//! Some leaf modules in the OS layer (`os::app::exit`,
//! `os::mod::dispatch` plugin route, `os::window_cmd::center`) need to
//! call into the higher app layer (cleanup, plugins, window helpers).
//! Once the host is split into `tynd-host-os` ⊥ `tynd-host-app`, those
//! direct `crate::cleanup`, `crate::plugins`, `crate::window` calls
//! would create an upward dependency cycle.
//!
//! Solution: each cycle is broken by a `OnceLock<Box<dyn Fn(...)>>`
//! living in `tynd-host-core` (the one crate everybody can depend on).
//! `tynd-host-app::runtime::run` registers concrete implementations
//! before the event loop drains its first message; `tynd-host-os` calls
//! them through getters that no-op when nothing is registered (the case
//! during `tynd-host-os` lib tests, which never spin the runtime).
//!
//! Nothing here is hot-path: each hook fires on shutdown, on plugin
//! dispatch (~µs cost vs serde_json round-trip), or on a window center
//! request (rare, user-driven).

use std::sync::OnceLock;

use serde_json::Value;

// ─────────────────────────── cleanup::run ──────────────────────────
//
// `os::app` calls this on `app.exit()` / `app.relaunch()` so registered
// temp dirs / sockets / file locks are released before
// `std::process::exit`. When the hook is unset (lib tests, plugin-only
// embeddings), we fall through silently — there's nothing to clean up.

type CleanupHook = Box<dyn Fn() + Send + Sync + 'static>;
static CLEANUP_HOOK: OnceLock<CleanupHook> = OnceLock::new();

/// Register the host's `cleanup::run` implementation. Called once by
/// `tynd-host-app::runtime::run` before the event loop spins up.
/// Idempotent — extra calls are silently ignored.
pub fn set_cleanup_hook<F>(f: F)
where
    F: Fn() + Send + Sync + 'static,
{
    let _ = CLEANUP_HOOK.set(Box::new(f));
}

/// Invoke the registered cleanup hook, if any. No-op when unset.
pub fn run_cleanup() {
    if let Some(f) = CLEANUP_HOOK.get() {
        f();
    }
}

// ─────────────────────────── plugin dispatch ───────────────────────
//
// `os::dispatch` routes `api == "plugin"` calls into the plugin
// registry. The registry lives in `tynd-host-app::plugins`, so the OS
// layer can't reference it directly. This hook lets the app layer
// inject a `(method, args) -> Result<Value, String>` closure that
// performs the registry lookup + dispatch.

type PluginDispatchHook =
    Box<dyn Fn(&str, Value) -> Result<Value, String> + Send + Sync + 'static>;
static PLUGIN_DISPATCH_HOOK: OnceLock<PluginDispatchHook> = OnceLock::new();

/// Register the host's plugin-dispatch implementation. Called once by
/// `tynd-host-app::runtime::run`. Idempotent.
pub fn set_plugin_dispatch_hook<F>(f: F)
where
    F: Fn(&str, Value) -> Result<Value, String> + Send + Sync + 'static,
{
    let _ = PLUGIN_DISPATCH_HOOK.set(Box::new(f));
}

/// Dispatch a plugin call. Returns a structured error if no host has
/// registered a dispatcher (e.g. lib tests, plugin-less builds).
pub fn dispatch_plugin(method: &str, args: Value) -> Result<Value, String> {
    match PLUGIN_DISPATCH_HOOK.get() {
        Some(f) => f(method, args),
        None => Err(format!(
            "plugin.{method}: no plugin dispatcher registered (host-app runtime not started)"
        )),
    }
}

// ─────────────────────────── window center ─────────────────────────
//
// `os::window_cmd` calls `window::center_window` to centre the active
// window on its current monitor. The `window` module (config/build
// helpers) lives in `tynd-host-app`. We pass the `&Window` through a
// `dyn Any` to avoid pulling `tao::window::Window` into `host-core`
// (`tao` is a heavy compile dep). The app-side hook is responsible for
// the downcast back to `&tao::window::Window`.

type WindowCenterHook =
    Box<dyn Fn(&dyn std::any::Any) + Send + Sync + 'static>;
static WINDOW_CENTER_HOOK: OnceLock<WindowCenterHook> = OnceLock::new();

/// Register the host's `window::center_window` implementation. Called
/// once by `tynd-host-app::runtime::run`. Idempotent.
pub fn set_window_center_hook<F>(f: F)
where
    F: Fn(&dyn std::any::Any) + Send + Sync + 'static,
{
    let _ = WINDOW_CENTER_HOOK.set(Box::new(f));
}

/// Centre the given `tao::window::Window` (passed as `&dyn Any`). No-op
/// when no hook is registered.
pub fn center_window(win: &dyn std::any::Any) {
    if let Some(f) = WINDOW_CENTER_HOOK.get() {
        f(win);
    }
}
