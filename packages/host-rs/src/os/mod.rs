// Always-on modules — structural glue (lifecycle, event bus, internal
// helpers) that the host relies on regardless of which user-facing APIs
// are compiled in.
pub mod app;
pub mod call_pool;
pub mod events;
pub mod icon;
pub mod os_info;
// Internal handle registry — shared by the per-API session/handle
// modules (websocket sessions, process children, fs watchers,
// shortcuts) so they don't each re-roll the same `OnceLock<DashMap +
// AtomicU64 NEXT_ID>` pattern. Crate-private; not exposed to plugins
// (the plugin-sdk has its own `Registry<T>` for the
// `Mutex<T>`-wrapped variant the SDK contract needs).
pub(crate) mod registry;
pub mod security;
pub mod window_cmd;

// Gatable user-facing OS APIs. Each feature flag flips both the module
// and its `dispatch` arm. Preset builds enable `api-all` (default);
// custom builds pick a subset.
#[cfg(feature = "api-acl")]
pub mod acl;
#[cfg(feature = "api-autolaunch")]
pub mod autolaunch;
#[cfg(feature = "api-biometric")]
pub mod biometric;
#[cfg(feature = "api-clipboard")]
pub mod clipboard;
#[cfg(feature = "api-cli")]
pub mod cli;
#[cfg(feature = "api-compute")]
pub mod compute;
#[cfg(feature = "api-keys")]
pub mod keys;
#[cfg(feature = "api-deep-link")]
pub mod deep_link;
#[cfg(feature = "api-dialog")]
pub mod dialog;
#[cfg(feature = "api-fs")]
pub mod fs;
#[cfg(feature = "api-http")]
pub mod http;
#[cfg(feature = "api-keyring")]
pub mod keyring;
#[cfg(feature = "api-log")]
pub mod log;
#[cfg(feature = "api-network")]
pub mod network;
#[cfg(feature = "api-notification")]
pub mod notification;
#[cfg(feature = "api-power")]
pub mod power;
#[cfg(feature = "api-process")]
pub mod process;
#[cfg(feature = "api-screenshot")]
pub mod screenshot;
#[cfg(feature = "api-shell")]
pub mod shell;
#[cfg(feature = "api-shortcuts")]
pub mod shortcuts;
#[cfg(feature = "api-sidecar")]
pub mod sidecar;
#[cfg(feature = "api-single-instance")]
pub mod single_instance;
#[cfg(feature = "api-store")]
pub mod store;
#[cfg(feature = "api-system-stats")]
pub mod system_stats;
#[cfg(feature = "api-theme")]
pub mod theme;
#[cfg(feature = "api-updater")]
pub mod updater;
#[cfg(feature = "api-vault")]
pub mod vault;
#[cfg(feature = "api-websocket")]
pub mod websocket;
#[cfg(feature = "api-window-state")]
pub mod window_persistence;

#[cfg(feature = "embedded-js")]
pub mod workers;

// === Tauri-parity additions (V1) ===
//
// Splash screen — CPU-buffer pre-window via softbuffer. Not an IPC
// API; consumed directly by the launcher's startup path before the
// webview spawns. Module is `#[cfg(feature = "splash-screen")]`
// internally, so this re-export resolves to nothing on default builds.
#[cfg(feature = "splash-screen")]
pub mod splash;

// AppleScript runner — macOS only, behind `api-apple-script`.
// Dispatched as the `appleScript` API (camelCase to match the JS-side
// proxy); the dispatch arm in the table below short-circuits to
// `api_disabled("apple-script")` on Linux/Windows.
#[cfg(all(target_os = "macos", feature = "api-apple-script"))]
pub mod apple_script;

// Swift↔Rust bridge smoke test — macOS only, behind `swift-bridge`.
// Re-exported so plugin authors can `use tynd_host::os::macos_swift`
// when they want to extend with their own `swift!` declarations.
#[cfg(all(target_os = "macos", feature = "swift-bridge"))]
pub mod macos_swift;

use serde_json::Value;

/// Produced whenever an API name is known but the feature wasn't
/// compiled in. Shape matches normal dispatch errors so frontends don't
/// need a special case. Only referenced by the `#[cfg(not(feature = ...))]`
/// branches, so it's dead code under `default` (= `api-all`).
///
/// `#[cold]` (B21): the disabled-API path is rare relative to the
/// normal dispatch hot path. Marking it cold lets the compiler optimise
/// it for size and place it out of the hot icache region — every
/// `Err(api_disabled(...))` arm in the macro-generated dispatch table
/// inherits the same locality benefit.
#[cold]
#[allow(dead_code)]
fn api_disabled(api: &str) -> String {
    format!(
        "OS API '{api}' is not compiled in this build. Enable the matching `api-{api}` feature \
         (or add it to `apis` in tynd.config.ts)."
    )
}

/// Generates the `api → cfg-gated module::dispatch` arms in the
/// outer `match`. Each row is `("name", "feature", module, "label")`:
/// the macro emits the literal arm with one `#[cfg(feature = "feature")]`
/// branch returning `module::dispatch(...)` and a sibling
/// `#[cfg(not(feature = "feature"))]` branch returning the
/// `api_disabled("label")` error string. The label is intentionally
/// the user-friendly name (e.g. `"deep-link"`, `"system-stats"`) so
/// the disabled message reads the same way the deployer wrote the
/// corresponding `apis: [...]` entry in `tynd.config.ts`.
///
/// Replaces 28 hand-written `match` arms (~280 LOC) with a single
/// declarative table.
///
/// V1 (B24): every `module::dispatch` arm receives `args: &Value` —
/// the public [`dispatch`] entry point takes ownership for the plugin
/// fast path (`crate::plugins::dispatch(method, args)`) but the OS
/// dispatch surface remains by-reference because most modules read
/// from `args` multiple times and the per-arm clone the previous
/// `Value::clone` paid is moot once we've taken ownership higher up.
macro_rules! dispatch_apis {
    (
        $api:ident, $method:ident, $args:ident,
        [ $( $name:literal => $feat:literal => $module:ident => $label:literal ),* $(,)? ]
        $( , $extra_arm:tt )* $(,)?
    ) => {
        match $api {
            $(
                $name => {
                    #[cfg(feature = $feat)]
                    { $module::dispatch($method, $args) }
                    #[cfg(not(feature = $feat))]
                    { Err(api_disabled($label)) }
                },
            )*
            $( $extra_arm )*
            _ => Err(format!("Unknown OS API: '{}'", $api)),
        }
    };
}

/// Dispatch a non-window OS API call from a background thread.
/// Returns `Ok(value)` or `Err(user-facing message)`.
///
/// V1 (B24): `args` is taken by value. The plugin path (`api == "plugin"`)
/// hands ownership straight to the plugin's `dispatch` — eliminates the
/// `args.clone()` the host used to pay before V1. OS modules see the
/// arg tree through a borrow; their dispatch arms are read-mostly and
/// rarely benefit from owning the value, so we hand them `&args` and
/// keep the per-module signature unchanged.
pub fn dispatch(api: &str, method: &str, args: Value) -> Result<Value, String> {
    // Always-on arms (no `api-*` feature gate — these crates are
    // structural). The `dispatch_apis!` macro then expands the rest.
    match api {
        "app" => return app::dispatch(method, &args),
        "os" => return os_info::dispatch(method, &args),
        "security" => return security::dispatch(method, &args),
        // Native plugins (api-* features irrelevant — plugin registry is
        // populated at process startup from the CLI-generated main.rs).
        // Hand ownership of `args` to the plugin — B24 eliminates the
        // historical `args.clone()` here.
        "plugin" => return crate::plugins::dispatch(method, args),
        // `workers` has bespoke semantics — present in `embedded-js`
        // builds, hard-error-with-hint everywhere else. Stays out of
        // the table because the disabled message isn't an
        // `api_disabled(...)` template.
        #[cfg(feature = "embedded-js")]
        "workers" => return workers::dispatch(method, &args),
        #[cfg(not(feature = "embedded-js"))]
        "workers" => {
            return Err(
                "workers API is lite-only; in full runtime use Bun.Worker natively".to_string(),
            );
        },
        // AppleScript — macOS only. Bespoke arm because the gate is
        // `cfg(target_os = "macos")` *and* `feature = "api-apple-script"`,
        // which the `dispatch_apis!` macro can't express in a single
        // row (the macro only handles a single feature gate).
        #[cfg(all(target_os = "macos", feature = "api-apple-script"))]
        "appleScript" => return apple_script::dispatch(method, &args),
        #[cfg(not(all(target_os = "macos", feature = "api-apple-script")))]
        "appleScript" => return Err(api_disabled("apple-script")),
        _ => {},
    }

    let args = &args;
    dispatch_apis!(
        api, method, args,
        [
            "acl"            => "api-acl"             => acl                 => "acl",
            "autolaunch"     => "api-autolaunch"      => autolaunch          => "autolaunch",
            "biometric"      => "api-biometric"       => biometric           => "biometric",
            "clipboard"      => "api-clipboard"       => clipboard           => "clipboard",
            "keys"           => "api-keys"            => keys                => "keys",
            "cli"            => "api-cli"             => cli                 => "cli",
            "compute"        => "api-compute"         => compute             => "compute",
            "deepLink"       => "api-deep-link"       => deep_link           => "deep-link",
            "dialog"         => "api-dialog"          => dialog              => "dialog",
            "fs"             => "api-fs"              => fs                  => "fs",
            "http"           => "api-http"            => http                => "http",
            "keyring"        => "api-keyring"         => keyring             => "keyring",
            "log"            => "api-log"             => log                 => "log",
            "network"        => "api-network"         => network             => "network",
            "notification"   => "api-notification"    => notification        => "notification",
            "power"          => "api-power"           => power               => "power",
            "process"        => "api-process"         => process             => "process",
            "screenshot"     => "api-screenshot"      => screenshot          => "screenshot",
            "shell"          => "api-shell"           => shell               => "shell",
            "shortcuts"      => "api-shortcuts"       => shortcuts           => "shortcuts",
            "sidecar"        => "api-sidecar"         => sidecar             => "sidecar",
            "singleInstance" => "api-single-instance" => single_instance     => "singleInstance",
            "store"          => "api-store"           => store               => "store",
            "systemStats"    => "api-system-stats"    => system_stats        => "system-stats",
            "theme"          => "api-theme"           => theme               => "theme",
            "updater"        => "api-updater"         => updater             => "updater",
            "vault"          => "api-vault"           => vault               => "vault",
            "websocket"      => "api-websocket"       => websocket           => "websocket",
            "windowState"    => "api-window-state"    => window_persistence  => "window-state",
        ]
    )
}
