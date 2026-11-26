//! Host OS APIs and the single table that declares them.
//!
//! Every API is one `mod` line and one row of `host_apis!` below, plus one
//! `API` descriptor ([`HostApi`]) in its module. The row emits the
//! [`HOST_APIS`] entry and the disabled-API error; [`dispatch`],
//! [`shutdown_resources`] and [`release_webview`] only walk the table.
//!
//! Naming rule (checked by `tests::rows_follow_the_naming_rule`): for an API
//! word `<w>`, the Cargo feature is `api-<kebab w>`, the module is
//! `<snake w>` and the wire name is `<camel w>`.

// API modules, one per table row that names a module of `api/`, under the cfg
// of its row. They are plain source lines, not macro output, so rustfmt
// follows them into the files. A row without its line does not compile when
// the feature is on; `tests::module_lines_match_the_table` checks the rest
// (a line without a row, a cfg that differs from the row).
// <api-modules>
pub(crate) mod app;
#[cfg(feature = "api-autolaunch")]
pub(crate) mod autolaunch;
#[cfg(feature = "api-biometric")]
pub(crate) mod biometric;
#[cfg(feature = "api-cli")]
pub(crate) mod cli;
#[cfg(feature = "api-clipboard")]
pub(crate) mod clipboard;
#[cfg(feature = "api-compute")]
pub(crate) mod compute;
#[cfg(feature = "api-deep-link")]
pub(crate) mod deep_link;
#[cfg(feature = "api-dialog")]
pub(crate) mod dialog;
#[cfg(feature = "api-fs")]
pub(crate) mod fs;
#[cfg(feature = "api-http")]
pub(crate) mod http;
#[cfg(feature = "api-keyring")]
pub(crate) mod keyring;
#[cfg(feature = "api-keys")]
pub(crate) mod keys;
pub(crate) mod log;
#[cfg(feature = "api-network")]
pub(crate) mod network;
#[cfg(feature = "api-notification")]
pub(crate) mod notification;
pub(crate) mod os_info;
#[cfg(feature = "api-power")]
pub(crate) mod power;
#[cfg(feature = "api-process")]
pub(crate) mod process;
#[cfg(feature = "api-screenshot")]
pub(crate) mod screenshot;
#[cfg(feature = "api-shell")]
pub(crate) mod shell;
#[cfg(feature = "api-shortcuts")]
pub(crate) mod shortcuts;
#[cfg(feature = "api-sidecar")]
pub(crate) mod sidecar;
#[cfg(feature = "api-single-instance")]
pub(crate) mod single_instance;
#[cfg(feature = "api-store")]
pub(crate) mod store;
#[cfg(feature = "api-system-stats")]
pub(crate) mod system_stats;
#[cfg(feature = "api-theme")]
pub(crate) mod theme;
#[cfg(feature = "api-updater")]
pub(crate) mod updater;
#[cfg(feature = "api-vault")]
pub(crate) mod vault;
#[cfg(feature = "api-websocket")]
pub(crate) mod websocket;
#[cfg(feature = "api-window-state")]
pub(crate) mod window_state;
// </api-modules>

// Menu and tray are real modules with their `api-*` feature; otherwise
// `menu_disabled.rs` / `tray_disabled.rs` supply the same crate-private
// surface as stubs so the event loop needs no cfg. Their commands run on the
// event loop (`event_loop` rows of the table).
#[cfg(feature = "api-menu")]
pub(crate) mod menu;
#[cfg(not(feature = "api-menu"))]
#[path = "api/menu_disabled.rs"]
pub(crate) mod menu;
#[cfg(feature = "api-tray")]
pub(crate) mod tray;
#[cfg(not(feature = "api-tray"))]
#[path = "api/tray_disabled.rs"]
pub(crate) mod tray;
// The `window` commands run on the event loop with a `&Window`, outside the
// table.
pub(crate) mod window_cmd;

// Helpers shared by several API modules (not APIs).
#[cfg(any(feature = "api-http", feature = "api-updater"))]
mod progress;
// Integer-keyed handle registry (WebSocket sessions).
#[cfg(feature = "api-websocket")]
mod registry;
#[cfg(any(feature = "api-clipboard", feature = "api-theme"))]
mod watch_refs;

use serde_json::Value;

use crate::{
    ipc::{call_pool, owner},
    security::acl,
};

/// Result of every host API call: a JSON value or a `"<api>.<method>: <msg>"`
/// error string.
pub(crate) type ApiResult = Result<Value, String>;

/// Webview-attributed entry of an API: `None` when `method` is not one of its
/// webview-scoped methods.
pub(crate) type WebviewDispatch =
    fn(method: &str, args: &Value, webview: &str) -> Option<ApiResult>;

/// One host API, declared by its module as `pub(crate) const API`.
pub(crate) struct HostApi {
    /// Wire name: the `api` of an IPC call and the prefix of its capability
    /// key. Must equal the name of the module's table row (compile-time check
    /// in `host_apis!`).
    pub(crate) name: &'static str,
    /// Run `method` with `args`. `args` is owned so an API can deserialize it
    /// without cloning the JSON tree.
    pub(crate) dispatch: fn(method: &str, args: Value) -> ApiResult,
    /// Methods that must know the calling webview (ref-counted watchers a
    /// reload or close has to release). Tried first for renderer calls; `None`
    /// from the function falls through to `dispatch`.
    pub(crate) dispatch_for_webview: Option<WebviewDispatch>,
    /// Release process-wide resources before the host exits.
    pub(crate) shutdown: Option<fn()>,
    /// Release (or re-own to the backend) what a webview's document left behind.
    pub(crate) release_webview: Option<fn(label: &str)>,
}

impl HostApi {
    /// A stateless API: no webview-scoped methods and nothing to release.
    pub(crate) const fn new(name: &'static str, dispatch: fn(&str, Value) -> ApiResult) -> Self {
        Self {
            name,
            dispatch,
            dispatch_for_webview: None,
            shutdown: None,
            release_webview: None,
        }
    }
}

/// One row of [`HOST_APIS`].
pub(crate) struct ApiRow {
    /// Wire name.
    pub(crate) name: &'static str,
    /// Cargo feature gating the API; `None` when it is always compiled.
    pub(crate) feature: Option<&'static str>,
    /// Whether the API is part of this build (its feature, and target, are on).
    pub(crate) compiled: bool,
    /// Descriptor served by [`dispatch`]. `None` when the API is not compiled,
    /// or when its commands need the event loop (window, menu bar, tray handle)
    /// and are routed by `app` instead.
    pub(crate) api: Option<&'static HostApi>,
    /// Module path named by the row (`stringify!` output), for the naming-rule
    /// and module-line tests.
    #[cfg(test)]
    module: Option<&'static str>,
    /// Target predicate of the row, beside its feature.
    #[cfg(test)]
    target: Option<&'static str>,
    /// Host-internal endpoint that is not an API of its own: absent from the
    /// catalog and exempt from the naming rule.
    #[cfg(test)]
    internal: bool,
}

impl ApiRow {
    /// Catalog name (`<kebab>` of `api-<kebab>`) used in the disabled-API error.
    fn catalog_name(&self) -> &'static str {
        self.feature
            .and_then(|feature| feature.strip_prefix("api-"))
            .unwrap_or(self.name)
    }
}

/// `a == b` in const context, for the row / descriptor name check.
const fn same_name(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if a[i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}

/// Declares every host API. Sections:
///
/// - `core`: always compiled. `"wire" => module`, where the module is one of
///   `api/` or a path (`crate::security::acl`).
/// - `gated`: behind a Cargo feature. `"wire" => module, "api-feature"`, with
///   an optional `if <cfg predicate>` for target-specific APIs.
/// - `event_loop`: gated APIs whose commands need the event loop and are
///   routed by `app`. `"wire" => "api-feature"`.
/// - `internal`: extra endpoints of a gated module that are not catalog APIs.
///   `"wire" => module::DESCRIPTOR, "api-feature"`.
///
/// Row order is the order of [`shutdown_resources`] and [`release_webview`].
/// Shutdown depends on it (see the comments in the table); the release of a
/// webview does not, every API releases only what it owns.
macro_rules! host_apis {
    (@target) => { None };
    (@target $extra:meta) => { Some(stringify!($extra)) };
    (
        core { $( $core_name:literal => $( $core_mod:ident )::+ ),* $(,)? }
        gated { $( $name:literal => $module:ident, $feat:literal $( if $extra:meta )? ),* $(,)? }
        event_loop { $( $loop_name:literal => $loop_feat:literal ),* $(,)? }
        internal {
            $( $int_name:literal => $int_mod:ident :: $int_api:ident, $int_feat:literal ),* $(,)?
        }
    ) => {
        $(
            const _: () = assert!(same_name($( $core_mod )::+::API.name, $core_name));
        )*
        $(
            #[cfg(all(feature = $feat $(, $extra)?))]
            const _: () = assert!(same_name($module::API.name, $name));
        )*
        $(
            #[cfg(feature = $int_feat)]
            const _: () = assert!(same_name($int_mod::$int_api.name, $int_name));
        )*

        /// The host API table: one row per API, compiled in or not.
        pub(crate) static HOST_APIS: &[ApiRow] = &[
            $( ApiRow {
                name: $core_name,
                feature: None,
                compiled: true,
                api: Some(&$( $core_mod )::+::API),
                #[cfg(test)]
                module: Some(stringify!($( $core_mod )::+)),
                #[cfg(test)]
                target: None,
                #[cfg(test)]
                internal: false,
            }, )*
            $( ApiRow {
                name: $name,
                feature: Some($feat),
                compiled: cfg!(all(feature = $feat $(, $extra)?)),
                api: {
                    #[cfg(all(feature = $feat $(, $extra)?))]
                    { Some(&$module::API) }
                    #[cfg(not(all(feature = $feat $(, $extra)?)))]
                    { None }
                },
                #[cfg(test)]
                module: Some(stringify!($module)),
                #[cfg(test)]
                target: host_apis!(@target $( $extra )?),
                #[cfg(test)]
                internal: false,
            }, )*
            $( ApiRow {
                name: $loop_name,
                feature: Some($loop_feat),
                compiled: cfg!(feature = $loop_feat),
                api: None,
                #[cfg(test)]
                module: None,
                #[cfg(test)]
                target: None,
                #[cfg(test)]
                internal: false,
            }, )*
            $( ApiRow {
                name: $int_name,
                feature: Some($int_feat),
                compiled: cfg!(feature = $int_feat),
                api: {
                    #[cfg(feature = $int_feat)]
                    { Some(&$int_mod::$int_api) }
                    #[cfg(not(feature = $int_feat))]
                    { None }
                },
                #[cfg(test)]
                module: None,
                #[cfg(test)]
                target: None,
                #[cfg(test)]
                internal: true,
            }, )*
        ];
    };
}

host_apis! {
    core {
        // The capability authority and logging are part of the host core,
        // never a build option.
        "acl" => crate::security::acl,
        "app" => app,
        "log" => log,
        // Wire name `os`; the module cannot be `api::os`.
        "os" => os_info,
        // Wire name `security`; the module is the policy authority.
        "security" => crate::security::policy,
    }
    gated {
        // Listeners first, so shutdown stops accepting forwarded launches and
        // URLs before the other APIs go away.
        "deepLink" => deep_link, "api-deep-link",
        "singleInstance" => single_instance, "api-single-instance",
        "autolaunch" => autolaunch, "api-autolaunch",
        "biometric" => biometric, "api-biometric",
        "cli" => cli, "api-cli",
        "clipboard" => clipboard, "api-clipboard",
        "compute" => compute, "api-compute",
        "dialog" => dialog, "api-dialog",
        "http" => http, "api-http",
        "keyring" => keyring, "api-keyring",
        "keys" => keys, "api-keys",
        "network" => network, "api-network",
        "notification" => notification, "api-notification",
        "power" => power, "api-power",
        "screenshot" => screenshot, "api-screenshot",
        "shell" => shell, "api-shell",
        "shortcuts" => shortcuts, "api-shortcuts",
        "sidecar" => sidecar, "api-sidecar",
        "systemStats" => system_stats, "api-system-stats",
        "theme" => theme, "api-theme",
        "updater" => updater, "api-updater",
        "vault" => vault, "api-vault",
        "websocket" => websocket, "api-websocket",
        "windowState" => window_state, "api-window-state",
        // Last three, in this order (`tests::shutdown_order_is_kept`). The
        // rows above that produce events (watchers, timers, sockets) stop
        // first; then the data sinks, so a backend handler reacting to a last
        // event can still write; child processes last, so a WebSocket to a
        // local sidecar closes cleanly before its peer is killed.
        "store" => store, "api-store",
        "fs" => fs, "api-fs",
        "process" => process, "api-process",
    }
    event_loop {
        // `menu` (or `menu_disabled.rs`).
        "menu" => "api-menu",
        // Secondary windows and child webviews, in `app::window_command`;
        // gated by `require_multiwindow`.
        "multiwindow" => "api-multiwindow",
        // `tray` (or `tray_disabled.rs`).
        "tray" => "api-tray",
    }
    internal {
        // Bind-address validation for backend plugins, under the `network`
        // feature. Capability names, not catalog APIs.
        "network-loopback" => network::LOOPBACK_API, "api-network",
        "network-public" => network::PUBLIC_API, "api-network",
    }
}

/// The table row for wire name `api`.
fn row(api: &str) -> Option<&'static ApiRow> {
    HOST_APIS.iter().find(|row| row.name == api)
}

/// Descriptors of the APIs compiled into this build, in table order.
fn enabled_apis() -> impl Iterator<Item = &'static HostApi> {
    HOST_APIS.iter().filter_map(|row| row.api)
}

/// Release process-wide OS resources before the host exits, in table order.
/// Every API that declares a `shutdown` in its [`HostApi`] is covered; nothing
/// is listed here.
pub(crate) fn shutdown_resources() {
    call_pool::shutdown();
    for api in enabled_apis() {
        if let Some(shutdown) = api.shutdown {
            shutdown();
        }
    }
}

/// Error for an `api: "plugin"` call in a host built without native plugins.
#[cfg(not(feature = "plugins"))]
#[cold]
fn plugins_disabled(method: &str) -> String {
    format!("plugin.{method}: no plugin dispatcher registered (native plugins are not initialized)")
}

/// Error string for known APIs with the feature gated out. `api` is the
/// catalog (kebab-case) name. Live in the default build (no `api-*` feature is
/// on by default) and under custom feature sets. Every gated API is fronted by
/// its `@tynd/plugin-<api>` official plugin, so the message names the package
/// to add. Crate-visible so the menu / tray stubs report the same message.
#[cold]
pub(crate) fn api_disabled(api: &str) -> String {
    format!(
        "OS API '{api}' is not compiled in this build. Install `@tynd/plugin-{api}` and add it to \
         `plugins` in tynd.config.ts (this builds a custom host with the `api-{api}` feature)."
    )
}

/// Dispatch a non-window OS API call from a background thread. `args` is
/// owned and handed to the API (or the plugin) without a clone.
pub fn dispatch(api: &str, method: &str, args: Value) -> Result<Value, String> {
    // Native plugins are not host APIs: `method` is `<plugin>:<command>` and
    // the plugin registry does its own routing.
    if api == "plugin" {
        #[cfg(feature = "plugins")]
        return crate::plugins::dispatch(method, args);
        #[cfg(not(feature = "plugins"))]
        return Err(plugins_disabled(method));
    }
    let Some(row) = row(api) else {
        return Err(format!("Unknown OS API: '{api}'"));
    };
    match row.api {
        Some(host_api) => (host_api.dispatch)(method, args),
        None if row.compiled => Err(format!(
            "OS API '{api}' is served by the event loop, not by the background dispatcher"
        )),
        None => Err(api_disabled(row.catalog_name())),
    }
}

/// Gate for secondary windows (`window.create` / `close` / `all`) and child
/// webviews (`webview.*`). Those commands are routed through the event loop
/// (`app::window_command`), not [`dispatch`], so they call this guard first and
/// reject with the same `api_disabled` message as every other gated API.
pub(crate) fn require_multiwindow() -> Result<(), String> {
    if row("multiwindow").is_some_and(|row| row.compiled) {
        Ok(())
    } else {
        Err(api_disabled("multiwindow"))
    }
}

/// `security` methods a renderer or a native plugin may call: read-only
/// policy evaluation. Anything else (today `configure`, and any method added
/// later) is denied by default outside the backend.
const UNTRUSTED_SECURITY_METHODS: &[&str] = &[
    "isFsAllowed",
    "checkFs",
    "isHttpAllowed",
    "checkHttp",
    "checkProcess",
    "checkAudioInput",
    "checkAudioOutput",
    "checkScreenCapture",
    "checkWindow",
];

/// `acl` methods a renderer or a native plugin may call: read-only inspection.
/// (`generateSchema` writes a file at a caller-chosen path: backend only.)
const UNTRUSTED_ACL_METHODS: &[&str] = &["isAllowed", "list"];

fn deny_privileged_mutation(api: &str, method: &str) -> Result<(), String> {
    // Security policy and the trusted updater endpoints / channel are
    // installed once by the backend; ACLs come from the startup frame only.
    // The policy authorities use an allowlist so a future mutation method is
    // never reachable from an untrusted caller by omission.
    let allowed = match api {
        "security" => UNTRUSTED_SECURITY_METHODS.contains(&method),
        "acl" => UNTRUSTED_ACL_METHODS.contains(&method),
        "updater" => method != "configure",
        _ => true,
    };
    if allowed {
        Ok(())
    } else {
        Err(format!(
            "{api}.{method}: privileged configuration is only accepted at backend startup"
        ))
    }
}

/// Dispatch an OS call originating in a native plugin.
///
/// Plugins may evaluate policy through the read-only `security` methods,
/// but cannot mutate the process-wide policy or ACL after startup.
#[cfg(feature = "plugins")]
pub(crate) fn dispatch_from_plugin(api: &str, method: &str, args: Value) -> Result<Value, String> {
    deny_privileged_mutation(api, method)?;
    dispatch(api, method, args)
}

/// Dispatch an OS call originating in a WebView, with distinct native-window
/// and webview identities.
///
/// Renderer code is an untrusted boundary: it may inspect the effective
/// policy, but it cannot mutate security/ACL state. Every renderer command
/// passes through the central capability authority with caller context.
///
/// Top-level webviews use the same label for both. Child webviews retain
/// their parent window label while capabilities can target their own label.
/// `generation` is the webview's document generation read when the IPC message
/// was received (see `owner::generation`), not when this pool thread runs it.
pub(crate) fn dispatch_webview_context(
    api: &str,
    method: &str,
    args: Value,
    window: &str,
    webview: &str,
    origin: Option<&str>,
    generation: u64,
) -> Result<Value, String> {
    deny_privileged_mutation(api, method)?;
    // The app version is the updater's downgrade floor, so only the backend
    // may declare it.
    if api == "app" && method == "setInfo" && args.get("version").is_some() {
        return Err(
            "app.setInfo: 'version' can only be set by the backend, not from a renderer".into(),
        );
    }

    let command = if api == "plugin" {
        let (plugin, plugin_method) = method.split_once(':').ok_or_else(|| {
            format!("renderer plugin method '{method}' must use '<plugin>:<command>'")
        })?;
        tynd_capabilities::plugin_command_key(plugin, plugin_method)?
    } else {
        format!("{api}:{method}")
    };
    if !acl::authority().is_allowed(&command, Some(window), Some(webview), origin) {
        return Err(format!(
            "renderer command '{command}' denied by capability policy"
        ));
    }

    // Ref-counted singleton watchers (clipboard monitor, theme watcher) are
    // attributed to the calling webview so a reload or close can release what
    // it left running.
    let for_webview = row(api)
        .and_then(|row| row.api)
        .and_then(|host_api| host_api.dispatch_for_webview);
    if let Some(for_webview) = for_webview {
        if let Some(result) = for_webview(method, &args, webview) {
            return result;
        }
    }

    // Resources created by this call (watches, file handles, processes, sockets,
    // keep-awake requests, shortcuts, scheduled notifications, staged updates)
    // record the webview, its document generation and the calling frame's origin
    // as their owner; other webviews, other frames of the same webview and later
    // documents cannot reach them. `origin` is the effective origin (`None` for
    // app-local content), so a remote document is also refused backend-owned
    // resources.
    owner::with_caller_at(webview, generation, origin, || dispatch(api, method, args))
}

/// Drop OS resources a webview holds when it is closed or its document is
/// replaced (reload / navigation): its clipboard-monitor and theme-watcher
/// references, and the fs watches and handles, sockets, keep-awake requests and
/// global shortcuts it opened (the JS that would
/// close them is gone). Spawned processes, scheduled notifications and staged
/// updates are not stopped but re-owned to the backend, so their events no
/// longer go to the label's next document. The label's document generation is
/// bumped first, which drops its already-queued webview-scoped events and makes
/// a resource that is still being created notice, once it is registered, that
/// its document is gone and release itself. Safe to call for any label, any
/// number of times. Every API that declares a `release_webview` in its
/// [`HostApi`] is covered.
pub(crate) fn release_webview(label: &str) {
    // First: a creation that registers after this point sees the new generation
    // and releases itself, and one that registered before is found by the
    // releases below. Events carry their resource's creation generation, so a
    // re-owned resource cannot reach the next document through the old one.
    owner::bump_generation(label);
    for api in enabled_apis() {
        if let Some(release) = api.release_webview {
            release(label);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    /// Structural APIs that are always compiled and have no catalog entry.
    const UNCATALOGUED_CORE: &[&str] = &["app", "os", "security"];

    /// `key = value` lines of one section of the crate manifest.
    fn manifest_section(header: &str) -> BTreeMap<String, String> {
        let manifest = include_str!("../Cargo.toml");
        let mut entries = BTreeMap::new();
        let mut inside = false;
        for line in manifest.lines().map(str::trim) {
            if line.starts_with('[') {
                inside = line == header;
            } else if inside && !line.starts_with('#') {
                if let Some((key, value)) = line.split_once('=') {
                    entries.insert(key.trim().to_owned(), value.trim().to_owned());
                }
            }
        }
        assert!(!entries.is_empty(), "no `{header}` section in Cargo.toml");
        entries
    }

    #[test]
    fn untrusted_callers_only_reach_allowlisted_policy_methods() {
        for (api, method) in [
            ("security", "configure"),
            ("updater", "configure"),
            ("acl", "configure"),
            // Writes a file at a caller-chosen path.
            ("acl", "generateSchema"),
            // Future mutation methods are denied by default.
            ("security", "setPolicy"),
            ("acl", "grant"),
            // Runtime scope grants are backend only.
            ("security", "allowPath"),
            ("security", "allowUrl"),
            ("security", "revokePath"),
            ("security", "revokeUrl"),
            ("security", "listGrants"),
        ] {
            assert!(
                deny_privileged_mutation(api, method).is_err(),
                "{api}.{method} must be denied"
            );
        }
        for method in UNTRUSTED_SECURITY_METHODS {
            assert!(deny_privileged_mutation("security", method).is_ok());
        }
        for method in UNTRUSTED_ACL_METHODS {
            assert!(deny_privileged_mutation("acl", method).is_ok());
        }
        assert!(deny_privileged_mutation("updater", "check").is_ok());
        assert!(deny_privileged_mutation("fs", "configure").is_ok());
    }

    /// Every allowlisted method is still served by its dispatch table, so the
    /// allowlist cannot drift into naming removed methods.
    #[test]
    fn policy_allowlists_name_dispatched_methods() {
        // `Value::Null` args stop every method at its argument check (or a
        // pure read), so nothing is written.
        for (host_api, methods) in [
            (&crate::security::policy::API, UNTRUSTED_SECURITY_METHODS),
            (&acl::API, UNTRUSTED_ACL_METHODS),
        ] {
            for method in methods {
                let unknown = crate::ipc::error::IpcError::unknown_method(host_api.name, method)
                    .into_string();
                let result = (host_api.dispatch)(method, Value::Null);
                assert_ne!(result.err(), Some(unknown), "{}.{method}", host_api.name);
            }
        }
    }

    fn public_rows() -> impl Iterator<Item = &'static ApiRow> {
        HOST_APIS.iter().filter(|row| !row.internal)
    }

    /// Last segment of the module path a row names.
    fn module_name(row: &ApiRow) -> Option<String> {
        let path = row.module?;
        let last = path.rsplit("::").next().unwrap_or(path);
        Some(last.trim().to_owned())
    }

    fn camel(kebab: &str) -> String {
        let mut out = String::new();
        let mut upper = false;
        for ch in kebab.chars() {
            if ch == '-' {
                upper = true;
            } else if upper {
                out.push(ch.to_ascii_uppercase());
                upper = false;
            } else {
                out.push(ch);
            }
        }
        out
    }

    #[test]
    fn api_disabled_names_the_feature() {
        let message = api_disabled("tray");
        assert!(message.contains("'tray'"));
        assert!(message.contains("`api-tray`"));
        assert!(message.contains("`@tynd/plugin-tray`"));
    }

    #[test]
    fn row_names_are_unique() {
        for (index, row) in HOST_APIS.iter().enumerate() {
            assert!(
                HOST_APIS[..index]
                    .iter()
                    .all(|other| other.name != row.name),
                "duplicate table row '{}'",
                row.name
            );
        }
    }

    #[test]
    fn rows_follow_the_naming_rule() {
        for row in public_rows() {
            let Some(feature) = row.feature else {
                // Core rows: the module is the wire name, except `os` and
                // `security` (the policy authority of `crate::security`).
                let module = match row.name {
                    "os" => "os_info",
                    "security" => "policy",
                    name => name,
                };
                assert_eq!(module_name(row).as_deref(), Some(module));
                continue;
            };
            let kebab = feature
                .strip_prefix("api-")
                .unwrap_or_else(|| panic!("feature '{feature}' of '{}' is not `api-*`", row.name));
            assert_eq!(row.name, camel(kebab), "wire name of `{feature}`");
            if let Some(module) = module_name(row) {
                assert_eq!(module, kebab.replace('-', "_"), "module of `{feature}`");
            }
        }
    }

    /// `(cfg predicate, module)` of every `mod` line between the
    /// `<api-modules>` markers of this file.
    fn module_lines() -> Vec<(Option<String>, String)> {
        let source = include_str!("api.rs");
        let start = source
            .find("// <api-modules>")
            .expect("opening marker of the module lines");
        let end = source
            .find("// </api-modules>")
            .expect("closing marker of the module lines");
        let mut lines = Vec::new();
        let mut cfg = None;
        for line in source[start..end].lines().skip(1) {
            if let Some(predicate) = line
                .strip_prefix("#[cfg(")
                .and_then(|rest| rest.strip_suffix(")]"))
            {
                cfg = Some(predicate.to_owned());
            } else if let Some(module) = line
                .strip_prefix("pub(crate) mod ")
                .and_then(|rest| rest.strip_suffix(';'))
            {
                lines.push((cfg.take(), module.to_owned()));
            } else {
                panic!("unexpected line between the module markers: {line}");
            }
        }
        lines
    }

    #[test]
    fn module_lines_match_the_table() {
        // Rows whose module is a file of `api/`; a core row may name a path
        // into another part of the crate instead.
        let mut expected: Vec<(Option<String>, String)> = public_rows()
            .filter(|row| row.module.is_some_and(|module| !module.contains("::")))
            .filter_map(|row| {
                let cfg = match (row.feature, row.target) {
                    (None, _) => None,
                    (Some(feature), None) => Some(format!("feature = \"{feature}\"")),
                    (Some(feature), Some(target)) => {
                        Some(format!("all(feature = \"{feature}\", {target})"))
                    },
                };
                Some((cfg, module_name(row)?))
            })
            .collect();
        expected.sort_by(|a, b| a.1.cmp(&b.1));
        assert_eq!(module_lines(), expected);
    }

    #[test]
    fn shutdown_order_is_kept() {
        let position = |name: &str| {
            HOST_APIS
                .iter()
                .position(|row| row.name == name)
                .unwrap_or_else(|| panic!("no table row '{name}'"))
        };
        // Listeners of forwarded launches stop before every other gated API.
        let first_gated = HOST_APIS
            .iter()
            .position(|row| row.feature.is_some())
            .expect("a gated row");
        assert_eq!(position("deepLink"), first_gated);
        assert_eq!(position("singleInstance"), first_gated + 1);
        // Event sources, then data sinks, then child processes.
        let (store, fs, process) = (position("store"), position("fs"), position("process"));
        assert!(store < fs && fs < process);
        for (index, row) in HOST_APIS.iter().enumerate() {
            let has_shutdown = row.api.is_some_and(|api| api.shutdown.is_some());
            if has_shutdown && !["store", "fs", "process"].contains(&row.name) {
                assert!(
                    index < store,
                    "'{}' must shut down before `store`",
                    row.name
                );
            }
        }
    }

    #[test]
    fn table_matches_the_api_catalog() {
        let catalog = manifest_section("[package.metadata.tynd.apis]");
        for (name, meta) in &catalog {
            let rows: Vec<_> = public_rows()
                .filter(|row| row.catalog_name() == name)
                .collect();
            assert_eq!(rows.len(), 1, "catalog API '{name}' needs exactly one row");
            let core = meta.contains("core = true");
            assert_eq!(
                rows[0].feature.is_none(),
                core,
                "catalog API '{name}': core flag and feature gate disagree"
            );
        }
        for row in public_rows() {
            let name = row.catalog_name();
            assert!(
                catalog.contains_key(name)
                    || (row.feature.is_none() && UNCATALOGUED_CORE.contains(&name)),
                "table row '{name}' is not in [package.metadata.tynd.apis]"
            );
        }
    }

    #[test]
    fn every_api_feature_has_a_row() {
        let features = manifest_section("[features]");
        let api_features: Vec<_> = features
            .keys()
            .filter(|feature| feature.starts_with("api-") && *feature != "api-all")
            .collect();
        for feature in api_features {
            let rows = public_rows()
                .filter(|row| row.feature == Some(feature.as_str()))
                .count();
            assert_eq!(rows, 1, "feature `{feature}` needs exactly one table row");
        }
        for row in HOST_APIS {
            if let Some(feature) = row.feature {
                assert!(
                    features.contains_key(feature),
                    "row '{}' names the unknown feature `{feature}`",
                    row.name
                );
            }
        }
    }

    #[test]
    fn disabled_apis_reject_by_naming_their_plugin() {
        for row in HOST_APIS {
            if row.compiled {
                continue;
            }
            let error = dispatch(row.name, "anything", Value::Null).unwrap_err();
            assert!(
                error.contains(&format!("`@tynd/plugin-{}`", row.catalog_name())),
                "unexpected error: {error}"
            );
        }
    }

    #[test]
    fn unknown_api_is_rejected() {
        let error = dispatch("nope", "anything", Value::Null).unwrap_err();
        assert_eq!(error, "Unknown OS API: 'nope'");
    }

    #[test]
    fn multiwindow_guard_follows_feature() {
        let result = require_multiwindow();
        if cfg!(feature = "api-multiwindow") {
            assert!(result.is_ok());
        } else {
            assert!(result.unwrap_err().contains("`@tynd/plugin-multiwindow`"));
        }
    }
}
