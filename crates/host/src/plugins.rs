//! Native plugin registry.
//!
//! Plugins implementing [`tynd_plugin_sdk::TyndPlugin`] are registered at
//! process startup by `main.rs`. Every IPC call addressed to `api: "plugin"`
//! routes through this module.
//!
//! Each plugin gets its own [`PluginCx`] carrying the shared emit callback
//! plus an optional allow-list of core OS commands reachable via
//! [`PluginCx::os_call`] (filled from `plugins: [{ pkg, permissions: [...] }]`;
//! plugin `apis` entries become `api:*` selectors).
//!
//! Plugins can register before OR after [`install_emit`]; entries registered
//! first get their `init()` retroactively once the bus is wired. Duplicate
//! registrations are logged and skipped (returns false).
//!
//! Storage: `Arc<dyn TyndPlugin>` behind `ArcSwap<EntryMap>` so the
//! `dispatch` hot path is a single atomic load with no `RwLock` contention.
//! `dispatch` takes `args: Value` by value, no clone at the boundary.

use std::{
    panic,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, OnceLock,
    },
};

pub(crate) mod setup;

use arc_swap::ArcSwap;
use parking_lot::{Condvar, Mutex};
use serde_json::Value;
use tynd_capabilities::manifest_from_plugin_commands;
use tynd_capabilities::validate_plugin_name;
pub use tynd_plugin_sdk::CallerInfo;
pub(crate) use tynd_plugin_sdk::HostEvent;
use tynd_plugin_sdk::{EmitFn, OsDispatchFn, PluginCx, PluginSetup, TyndPlugin, UriHandler};

/// Per-plugin entry. Both the plugin handle and `PluginCx` are `Arc`-shared
/// so a snapshot swap is a refcount bump, not a deep clone.
#[derive(Clone)]
struct Entry {
    plugin: Arc<dyn TyndPlugin>,
    /// Monotonic insertion sequence used for deterministic introspection and
    /// shutdown. `HashMap` iteration order is intentionally unstable.
    registration_order: u64,
    /// `None` until [`install_emit`] wires the shared callbacks.
    cx: Option<Arc<PluginCx>>,
    /// Allow-list from registration; kept so we can rebuild `cx` if
    /// `install_emit` fires after this entry registered.
    allowed_permissions: Option<Vec<String>>,
    /// Set after `plugin.init()` runs. Guards against double-init.
    initialized: bool,
    /// Claimed under the writer lock before running user code outside it.
    /// Prevents concurrent registration / emitter installation from invoking
    /// the same plugin's `init()` twice.
    initializing: bool,
    /// Captures the error returned by `plugin.init()` so subsequent
    /// dispatch attempts can surface "plugin X init failed: <reason>"
    /// instead of the misleading default "not initialized" — the
    /// operator otherwise can't tell whether the plugin silently
    /// crashed at boot or was never wired.
    init_error: Option<String>,
    /// Claimed before invoking third-party shutdown code. Cleanup can be
    /// requested from several exit paths, but every plugin hook must run at
    /// most once.
    shutdown_started: bool,
}

/// Plugin map keyed by name (O(1) dispatch lookup).
type EntryMap = ahash::HashMap<&'static str, Entry>;

struct Registry {
    entries: ArcSwap<EntryMap>,
    /// Serializes writers; readers go through `entries.load()`.
    write_lock: Mutex<()>,
    /// Serializes shutdown batches. Concurrent callers wait; a reentrant call
    /// from the current plugin hook returns instead of deadlocking.
    shutdown_owner: Mutex<Option<std::thread::ThreadId>>,
    shutdown_complete: Condvar,
    default_emit: OnceLock<EmitFn>,
    default_dispatch: OnceLock<OsDispatchFn>,
}

impl Registry {
    fn new() -> Self {
        Self {
            entries: ArcSwap::new(Arc::new(EntryMap::default())),
            write_lock: Mutex::new(()),
            shutdown_owner: Mutex::new(None),
            shutdown_complete: Condvar::new(),
            default_emit: OnceLock::new(),
            default_dispatch: OnceLock::new(),
        }
    }
}

static REGISTRY: std::sync::LazyLock<Registry> = std::sync::LazyLock::new(Registry::new);

/// Forwards `PluginCx::register_init_script` / `register_uri_scheme` into the
/// [`setup`] registry that the webview builders read.
#[derive(Debug)]
struct HostSetup;

impl PluginSetup for HostSetup {
    fn init_script(&self, script: String) -> Result<(), String> {
        setup::register_init_script(script, false)
    }

    fn init_script_scoped(&self, script: String, remote: bool) -> Result<(), String> {
        setup::register_init_script(script, remote)
    }

    fn uri_scheme(&self, name: String, handler: UriHandler) -> Result<(), String> {
        setup::register_uri_scheme(name, handler, false)
    }

    fn uri_scheme_scoped(
        &self,
        name: String,
        handler: UriHandler,
        remote: bool,
    ) -> Result<(), String> {
        setup::register_uri_scheme(name, handler, remote)
    }
}

// ── Before-dispatch hook chain ───────────────────────────────────────────

/// What a [`BeforeDispatchHook`] sees about a plugin call.
#[derive(Debug)]
pub struct DispatchInfo<'a> {
    /// Plugin name.
    pub plugin: &'a str,
    /// Plugin command name.
    pub method: &'a str,
    /// Call arguments, before the plugin consumes them.
    pub args: &'a Value,
    /// Renderer that issued the call; `None` for backend or native callers.
    pub caller: Option<CallerInfo>,
}

/// Hook run before every plugin dispatch (audit, logging, rate limiting).
/// Returning `Err` rejects the call with that message; the plugin is not run
/// and later hooks are skipped. A panicking hook rejects the call too.
pub type BeforeDispatchHook = Arc<dyn Fn(&DispatchInfo<'_>) -> Result<(), String> + Send + Sync>;

type HookList = Vec<BeforeDispatchHook>;
static BEFORE_DISPATCH: std::sync::LazyLock<ArcSwap<HookList>> =
    std::sync::LazyLock::new(|| ArcSwap::from_pointee(Vec::new()));

/// Append a hook to the before-dispatch chain. Hooks run in registration
/// order; the chain is empty by default.
pub fn add_before_dispatch(hook: BeforeDispatchHook) {
    BEFORE_DISPATCH.rcu(|current| {
        let mut next = HookList::clone(current);
        next.push(Arc::clone(&hook));
        next
    });
}

fn run_before_dispatch(info: &DispatchInfo<'_>) -> Result<(), String> {
    let hooks = BEFORE_DISPATCH.load();
    for hook in hooks.iter() {
        match panic::catch_unwind(panic::AssertUnwindSafe(|| hook(info))) {
            Ok(Ok(())) => {},
            Ok(Err(reason)) => return Err(reason),
            Err(payload) => {
                let reason = panic_message(payload);
                tracing::error!(
                    plugin = ?info.plugin,
                    method = ?info.method,
                    error = ?reason,
                    "before-dispatch hook panicked"
                );
                return Err(format!(
                    "plugin '{}.{}' rejected: before-dispatch hook panicked",
                    info.plugin, info.method
                ));
            },
        }
    }
    Ok(())
}

/// Run `f` with `window` / `webview` / `origin` exposed to plugins through
/// `PluginCx::caller()` and to before-dispatch hooks. Wrap the renderer-driven
/// OS dispatch with it; `f` must run on the calling thread.
pub(crate) fn with_call_context<R>(
    window: &str,
    webview: &str,
    origin: Option<&str>,
    f: impl FnOnce() -> R,
) -> R {
    tynd_plugin_sdk::with_caller(
        Some(CallerInfo {
            window: window.to_string(),
            webview: webview.to_string(),
            origin: origin.map(str::to_string),
        }),
        f,
    )
}

// ── Host event broadcast ─────────────────────────────────────────────────

static APP_EXIT_BROADCAST: AtomicBool = AtomicBool::new(false);

/// Deliver `event` to every initialized plugin's `on_event`, in registration
/// order. A panicking plugin is logged and skipped. Runs on the calling thread
/// (usually the UI thread): plugins are expected to return quickly.
///
/// `AppExit` is also fired by [`shutdown_all`]; a second `AppExit` is dropped.
pub(crate) fn broadcast_event(event: &HostEvent) {
    if matches!(event, HostEvent::AppExit) && APP_EXIT_BROADCAST.swap(true, Ordering::AcqRel) {
        return;
    }
    let snapshot = REGISTRY.entries.load();
    let mut targets: Vec<_> = snapshot
        .iter()
        .filter_map(|(name, entry)| {
            let cx = entry.cx.as_ref()?;
            Some((*name, entry.registration_order, &entry.plugin, cx))
        })
        .collect();
    targets.sort_unstable_by_key(|(_, order, _, _)| *order);
    for (name, _, plugin, cx) in targets {
        if let Err(payload) =
            panic::catch_unwind(panic::AssertUnwindSafe(|| plugin.on_event(cx, event)))
        {
            tracing::error!(
                plugin = ?name,
                error = ?panic_message(payload),
                "plugin on_event panicked"
            );
        }
    }
}

/// RCU helper: clone the snapshot under the write lock, mutate, publish.
fn rcu(mutate: impl FnOnce(&mut EntryMap)) {
    rcu_with(|map| {
        mutate(map);
    });
}

/// RCU helper variant returning a value the caller computes inside the
/// write lock (e.g. the entries claimed for init or shutdown).
fn rcu_with<R>(mutate: impl FnOnce(&mut EntryMap) -> R) -> R {
    let _guard = REGISTRY.write_lock.lock();
    let current = REGISTRY.entries.load();
    let mut next = EntryMap::clone(&current);
    let r = mutate(&mut next);
    REGISTRY.entries.store(Arc::new(next));
    r
}

/// Install the shared emit callback + default OS dispatcher. Must run
/// before the first IPC message; can fire before or after plugin registration.
pub(crate) fn install_emit(emit: EmitFn) {
    // First install wins (tests can spin the app loop twice).
    let _ = REGISTRY.default_emit.set(emit);
    let dispatch: OsDispatchFn = Arc::new(|api: &str, method: &str, args: Value| {
        crate::api::dispatch_from_plugin(api, method, args).map_err(Into::into)
    });
    let _ = REGISTRY.default_dispatch.set(dispatch);

    initialize_pending();
}

type PendingPlugin = (&'static str, Option<Vec<String>>, Arc<dyn TyndPlugin>);

fn initialize_pending() {
    let (Some(emit), Some(dispatch)) = (
        REGISTRY.default_emit.get().cloned(),
        REGISTRY.default_dispatch.get().cloned(),
    ) else {
        return;
    };

    // Claim pending entries under the writer lock, then run third-party code
    // after releasing it. A plugin may register another plugin from `init()`;
    // keeping user code under this non-reentrant lock would deadlock.
    let pending: Vec<PendingPlugin> = rcu_with(|map| {
        map.iter_mut()
            .filter(|(_, entry)| !entry.initialized && !entry.initializing)
            .map(|(name, entry)| {
                entry.initializing = true;
                (
                    *name,
                    entry.allowed_permissions.clone(),
                    Arc::clone(&entry.plugin),
                )
            })
            .collect()
    });

    let results: Vec<_> = pending
        .into_iter()
        .map(|(name, allowed, plugin)| {
            let result = init_plugin(name, plugin.as_ref(), allowed, &emit, &dispatch);
            (name, result)
        })
        .collect();
    if results.is_empty() {
        return;
    }
    rcu(|map| {
        for (name, result) in results {
            let Some(entry) = map.get_mut(name) else {
                continue;
            };
            entry.initializing = false;
            match result {
                Ok(cx) => {
                    entry.cx = Some(cx);
                    entry.initialized = true;
                    entry.init_error = None;
                },
                Err(reason) => {
                    entry.cx = None;
                    entry.initialized = false;
                    entry.init_error = Some(reason);
                },
            }
        }
    });
}

/// Read-only `security` commands every plugin may call to evaluate policy.
/// Exact methods, so a future mutator does not become reachable merely
/// because it belongs to `security`.
const SECURITY_READ_PERMISSIONS: &[&str] = &[
    "security:isFsAllowed",
    "security:checkFs",
    "security:isHttpAllowed",
    "security:checkHttp",
    "security:checkProcess",
    "security:checkAudioInput",
    "security:checkAudioOutput",
    "security:checkScreenCapture",
    "security:checkWindow",
];

/// The declared allow-list plus [`SECURITY_READ_PERMISSIONS`], deduplicated.
fn effective_permissions(allowed: Option<Vec<String>>) -> Vec<String> {
    let mut permissions = allowed.unwrap_or_default();
    for permission in SECURITY_READ_PERMISSIONS {
        if !permissions.iter().any(|candidate| candidate == permission) {
            permissions.push((*permission).to_string());
        }
    }
    permissions
}

/// Run one plugin's `init` hook with a fresh [`PluginCx`]. A failing or
/// panicking hook is logged and returned as its reason: one bad plugin must
/// not bring down the host, and dispatch then reports
/// "plugin X init failed: <reason>" rather than a misleading "not initialized".
fn init_plugin(
    name: &'static str,
    plugin: &dyn TyndPlugin,
    allowed: Option<Vec<String>>,
    emit: &EmitFn,
    dispatch: &OsDispatchFn,
) -> Result<Arc<PluginCx>, String> {
    let cx = PluginCx::new(emit.clone())
        .with_dispatch(dispatch.clone())
        .with_setup(Arc::new(HostSetup))
        .with_allowed_permissions(effective_permissions(allowed));
    let init_result = panic::catch_unwind(panic::AssertUnwindSafe(|| plugin.init(&cx)));
    if let Err(e) = init_result.unwrap_or_else(|payload| {
        Err(format!("plugin init panicked: {}", panic_message(payload)).into())
    }) {
        let reason = e.to_string();
        // Render with `Debug` (`?`) — plugin name and arbitrary
        // error chains can carry control bytes (file paths, file
        // contents) that `Display` would splice as forged log lines.
        tracing::error!(plugin = ?name, error = ?reason, "plugin init failed");
        return Err(reason);
    }
    Ok(Arc::new(cx))
}

/// Add a plugin to the registry with no declared core-API capabilities.
/// The plugin can inspect read-only security policy but every other
/// [`PluginCx::os_call`] is denied. Use [`register_with_permissions`] to
/// grant explicit command selectors.
///
/// Returns `true` on success, `false` if the name was already registered
/// (duplicate registration is logged and skipped — the existing entry is
/// left untouched).
pub fn register(plugin: Arc<dyn TyndPlugin>) -> bool {
    register_with_permissions(plugin, None)
}

/// Add a plugin with an optional command allow-list.
///
/// Selectors use `api:method`; `api:*` grants every method on one API.
/// `None` and `Some(vec![])` both grant no generic host command. Exact
/// read-only `security` inspection methods are added internally.
///
/// Returns `true` on success, `false` if a plugin with the same name is
/// already registered. A duplicate is logged via `tracing::error!` and
/// the existing entry is preserved unchanged — registering a duplicate
/// must never panic the host.
///
/// A plugin whose `init` hook returns `Err` is logged via `tracing::error!`
/// and left with `initialized: false` so the host can still boot — one
/// bad plugin must not take the rest of the runtime down.
pub fn register_with_permissions(
    plugin: Arc<dyn TyndPlugin>,
    allowed_permissions: Option<Vec<String>>,
) -> bool {
    let name = plugin.name();
    if let Err(error) = validate_plugin_name(name) {
        tracing::error!(
            plugin = ?name,
            error = ?error,
            "plugin registration rejected invalid name",
        );
        return false;
    }
    let permission_manifest = match manifest_from_plugin_commands(name, plugin.command_names()) {
        Ok(manifest) => manifest,
        Err(error) => {
            tracing::error!(
                plugin = ?name,
                error = ?error,
                "plugin permission manifest generation failed",
            );
            return false;
        },
    };

    // Cheap-but-non-authoritative dup probe: skip the `init` side effects
    // when the name is *obviously* already taken. The authoritative check
    // happens in `publish_entry` under the writer lock, the only check that
    // serializes against concurrent registrations.
    if REGISTRY.entries.load().contains_key(name) {
        log_duplicate_registration(name);
        return false;
    }

    let entry = Entry {
        plugin,
        registration_order: 0,
        cx: None,
        allowed_permissions,
        initialized: false,
        initializing: false,
        init_error: None,
        shutdown_started: false,
    };
    let published = publish_entry(name, entry, permission_manifest);
    if !published {
        return false;
    }

    initialize_pending();
    wait_while_initializing(name);
    true
}

fn log_duplicate_registration(name: &str) {
    tracing::error!(
        plugin = %name,
        "duplicate plugin registration; keeping the existing entry",
    );
}

/// Insert `entry` (with the next registration order) unless `name` is taken.
///
/// The entry is published only after its ACL manifest commits. Holding the
/// registry writer lock makes duplicate detection + manifest install + entry
/// publication one transaction from dispatch readers' perspective.
fn publish_entry(
    name: &'static str,
    mut entry: Entry,
    permission_manifest: Option<tynd_capabilities::Manifest>,
) -> bool {
    let _guard = REGISTRY.write_lock.lock();
    let current = REGISTRY.entries.load();
    if current.contains_key(name) {
        log_duplicate_registration(name);
        return false;
    }

    if let Some(manifest) = permission_manifest {
        if let Err(error) = crate::security::acl::authority().register_manifest(name, manifest) {
            tracing::error!(
                plugin = ?name,
                error = ?error,
                "plugin permission manifest registration failed",
            );
            return false;
        }
    }

    let mut next = EntryMap::clone(&current);
    entry.registration_order = next
        .values()
        .map(|entry| entry.registration_order)
        .max()
        .unwrap_or(0)
        .saturating_add(1);
    next.insert(name, entry);
    REGISTRY.entries.store(Arc::new(next));
    true
}

/// Block until `name`'s `init` hook has finished.
///
/// Another registration thread may have claimed this entry inside its own
/// `initialize_pending` call. Registration is synchronous: do not return
/// while that plugin's init hook is still running. Reentrant registration
/// remains safe because a child entry is initialized by the current thread
/// before this wait is reached.
fn wait_while_initializing(name: &str) {
    while REGISTRY
        .entries
        .load()
        .get(name)
        .is_some_and(|entry| entry.initializing)
    {
        std::thread::yield_now();
    }
}

/// Dispatch `{type: "os_call", api: "plugin", method: "<plugin>:<method>"}`.
/// `args` is moved into the plugin (no clone at the boundary).
pub(crate) fn dispatch(method: &str, args: Value) -> Result<Value, String> {
    let (plugin_name, method_name) = method
        .split_once(':')
        .ok_or_else(|| format!("plugin call missing ':' separator: '{method}'"))?;

    let snapshot = REGISTRY.entries.load();
    let entry = snapshot
        .get(plugin_name)
        .ok_or_else(|| format!("unknown plugin: '{plugin_name}'"))?;

    let cx = entry.cx.as_ref().ok_or_else(|| {
        // Surface the init error if we captured one — distinguishes a
        // crashed-on-init plugin from a genuinely-not-yet-wired one so
        // operators don't misread "not initialized" as "you forgot to
        // enable it".
        if let Some(reason) = entry.init_error.as_deref() {
            format!("plugin '{plugin_name}' init failed: {reason}")
        } else {
            format!("plugin '{plugin_name}' not initialized (install_emit missing)")
        }
    })?;

    run_before_dispatch(&DispatchInfo {
        plugin: plugin_name,
        method: method_name,
        args: &args,
        caller: tynd_plugin_sdk::current_caller(),
    })?;

    panic::catch_unwind(panic::AssertUnwindSafe(|| {
        entry.plugin.dispatch(cx, method_name, args)
    }))
    .unwrap_or_else(|payload| {
        Err(format!(
            "plugin '{plugin_name}' dispatch panicked: {}",
            panic_message(payload)
        )
        .into())
    })
    .map_err(|e| e.to_string())
}

/// How many plugins are registered, for the debug protocol.
#[cfg(feature = "debug-protocol")]
pub(crate) fn count() -> usize {
    REGISTRY.entries.load().len()
}

/// Stable debug-protocol view of the registry. This intentionally exposes
/// lifecycle facts only; command schemas remain generated TypeScript assets.
#[cfg(feature = "debug-protocol")]
pub(crate) fn introspect() -> Vec<Value> {
    let snapshot = REGISTRY.entries.load();
    let mut entries: Vec<_> = snapshot
        .iter()
        .map(|(name, entry)| (*name, entry))
        .collect();
    entries.sort_unstable_by_key(|(_, entry)| entry.registration_order);
    entries
        .into_iter()
        .map(|(name, entry)| {
            serde_json::json!({
                "name": name,
                "initialized": entry.initialized,
                "initializing": entry.initializing,
                "initError": entry.init_error,
                "shutdownStarted": entry.shutdown_started,
                "allowedPermissions": entry.allowed_permissions.clone().unwrap_or_default(),
                "allowedApis": entry.allowed_permissions
                    .as_deref()
                    .unwrap_or_default()
                    .iter()
                    .filter_map(|permission| permission.strip_suffix(":*"))
                    .collect::<Vec<_>>(),
            })
        })
        .collect()
}

/// Run every plugin's `shutdown` hook in registration order. A failing
/// shutdown is logged with structured fields and does not stop the others.
pub(crate) fn shutdown_all() {
    if !acquire_shutdown_owner() {
        return;
    }
    let _release_shutdown = crate::util::defer(|| {
        let mut owner = REGISTRY.shutdown_owner.lock();
        *owner = None;
        REGISTRY.shutdown_complete.notify_all();
    });

    // Plugins learn about the exit before any plugin is torn down.
    broadcast_event(&HostEvent::AppExit);

    loop {
        let Some(mut entries) = claim_shutdown_batch() else {
            std::thread::yield_now();
            continue;
        };

        // Hooks were claimed under the writer lock; execute third-party code
        // after releasing it. Repeated cleanup is idempotent and a shutdown
        // hook may register another plugin without deadlocking.
        if entries.is_empty() {
            break;
        }

        entries.sort_unstable_by_key(|(_, registration_order, _, _)| *registration_order);
        for (name, _, plugin, cx) in entries {
            run_shutdown_hook(name, plugin.as_ref(), &cx);
        }
    }
}

/// Make the current thread the shutdown owner, waiting for any other
/// thread's batch to finish. `false` for a reentrant call from a hook the
/// current thread is already running.
fn acquire_shutdown_owner() -> bool {
    let current_thread = std::thread::current().id();
    let mut owner = REGISTRY.shutdown_owner.lock();
    loop {
        match *owner {
            None => {
                *owner = Some(current_thread);
                return true;
            },
            Some(active) if active == current_thread => return false,
            Some(_) => REGISTRY.shutdown_complete.wait(&mut owner),
        }
    }
}

type ShutdownTarget = (&'static str, u64, Arc<dyn TyndPlugin>, Arc<PluginCx>);

/// Mark every initialized plugin whose shutdown has not started and return
/// them. `None` while an init hook is still executing: later plugins must
/// not be torn down before it ends (registration is synchronous, so this
/// window is bounded by the plugin's own init hook).
fn claim_shutdown_batch() -> Option<Vec<ShutdownTarget>> {
    rcu_with(|map| {
        if map.values().any(|entry| entry.initializing) {
            return None;
        }
        Some(
            map.iter_mut()
                .filter_map(|(name, entry)| {
                    if entry.shutdown_started {
                        return None;
                    }
                    let cx = entry.cx.as_ref().map(Arc::clone)?;
                    entry.shutdown_started = true;
                    Some((
                        *name,
                        entry.registration_order,
                        Arc::clone(&entry.plugin),
                        cx,
                    ))
                })
                .collect(),
        )
    })
}

/// Run one plugin's `shutdown` hook, logging a failure or panic.
fn run_shutdown_hook(name: &str, plugin: &dyn TyndPlugin, cx: &PluginCx) {
    match panic::catch_unwind(panic::AssertUnwindSafe(|| plugin.shutdown(cx))) {
        Ok(Ok(())) => {},
        Ok(Err(error)) => {
            let reason = error.to_string();
            tracing::error!(plugin = ?name, error = ?reason, "plugin shutdown failed");
        },
        Err(payload) => {
            let reason = panic_message(payload);
            tracing::error!(plugin = ?name, error = ?reason, "plugin shutdown panicked");
        },
    }
}

fn panic_message(payload: Box<dyn std::any::Any + Send>) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_string()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "unknown panic payload".to_string()
    }
}

/// Default emit callback — routes through `crate::ipc::events::emit` so plugins share
/// the OS event bus.
pub(crate) fn default_emit_fn() -> EmitFn {
    Arc::new(|name: &str, payload: Value| {
        crate::ipc::events::emit(name, &payload);
    })
}

/// Install the default emit callback. Convenience wrapper matching what
/// a host built without native plugins wants.
pub(crate) fn install_default_emit() {
    install_emit(default_emit_fn());
}

#[cfg(test)]
mod tests;
