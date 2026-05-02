// Always-on modules — structural glue (lifecycle, event bus, internal
// helpers) that the host relies on regardless of which user-facing APIs
// are compiled in.
pub mod app;
pub mod call_pool;
pub mod events;
pub mod icon;
pub mod os_info;
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
#[cfg(feature = "api-compute")]
pub mod compute;
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

#[cfg(feature = "embedded-js")]
pub mod workers;

use serde_json::Value;

/// Produced whenever an API name is known but the feature wasn't
/// compiled in. Shape matches normal dispatch errors so frontends don't
/// need a special case. Only referenced by the `#[cfg(not(feature = ...))]`
/// branches, so it's dead code under `default` (= `api-all`).
#[allow(dead_code)]
fn api_disabled(api: &str) -> String {
    format!(
        "OS API '{api}' is not compiled in this build. Enable the matching `api-{api}` feature \
         (or add it to `apis` in tynd.config.ts)."
    )
}

/// Dispatch a non-window OS API call from a background thread.
/// Returns `Ok(value)` or `Err(user-facing message)`.
#[allow(clippy::too_many_lines)]
pub fn dispatch(api: &str, method: &str, args: &Value) -> Result<Value, String> {
    match api {
        "app" => app::dispatch(method, args),
        "os" => os_info::dispatch(method, args),
        "security" => security::dispatch(method, args),
        // Native plugins (api-* features irrelevant — plugin registry is
        // populated at process startup from the CLI-generated main.rs).
        "plugin" => crate::plugins::dispatch(method, args),

        "acl" => {
            #[cfg(feature = "api-acl")]
            {
                acl::dispatch(method, args)
            }
            #[cfg(not(feature = "api-acl"))]
            {
                Err(api_disabled("acl"))
            }
        },
        "autolaunch" => {
            #[cfg(feature = "api-autolaunch")]
            {
                autolaunch::dispatch(method, args)
            }
            #[cfg(not(feature = "api-autolaunch"))]
            {
                Err(api_disabled("autolaunch"))
            }
        },
        "biometric" => {
            #[cfg(feature = "api-biometric")]
            {
                biometric::dispatch(method, args)
            }
            #[cfg(not(feature = "api-biometric"))]
            {
                Err(api_disabled("biometric"))
            }
        },
        "clipboard" => {
            #[cfg(feature = "api-clipboard")]
            {
                clipboard::dispatch(method, args)
            }
            #[cfg(not(feature = "api-clipboard"))]
            {
                Err(api_disabled("clipboard"))
            }
        },
        "compute" => {
            #[cfg(feature = "api-compute")]
            {
                compute::dispatch(method, args)
            }
            #[cfg(not(feature = "api-compute"))]
            {
                Err(api_disabled("compute"))
            }
        },
        "dialog" => {
            #[cfg(feature = "api-dialog")]
            {
                dialog::dispatch(method, args)
            }
            #[cfg(not(feature = "api-dialog"))]
            {
                Err(api_disabled("dialog"))
            }
        },
        "fs" => {
            #[cfg(feature = "api-fs")]
            {
                fs::dispatch(method, args)
            }
            #[cfg(not(feature = "api-fs"))]
            {
                Err(api_disabled("fs"))
            }
        },
        "http" => {
            #[cfg(feature = "api-http")]
            {
                http::dispatch(method, args)
            }
            #[cfg(not(feature = "api-http"))]
            {
                Err(api_disabled("http"))
            }
        },
        "keyring" => {
            #[cfg(feature = "api-keyring")]
            {
                keyring::dispatch(method, args)
            }
            #[cfg(not(feature = "api-keyring"))]
            {
                Err(api_disabled("keyring"))
            }
        },
        "log" => {
            #[cfg(feature = "api-log")]
            {
                log::dispatch(method, args)
            }
            #[cfg(not(feature = "api-log"))]
            {
                Err(api_disabled("log"))
            }
        },
        "network" => {
            #[cfg(feature = "api-network")]
            {
                network::dispatch(method, args)
            }
            #[cfg(not(feature = "api-network"))]
            {
                Err(api_disabled("network"))
            }
        },
        "notification" => {
            #[cfg(feature = "api-notification")]
            {
                notification::dispatch(method, args)
            }
            #[cfg(not(feature = "api-notification"))]
            {
                Err(api_disabled("notification"))
            }
        },
        "power" => {
            #[cfg(feature = "api-power")]
            {
                power::dispatch(method, args)
            }
            #[cfg(not(feature = "api-power"))]
            {
                Err(api_disabled("power"))
            }
        },
        "process" => {
            #[cfg(feature = "api-process")]
            {
                process::dispatch(method, args)
            }
            #[cfg(not(feature = "api-process"))]
            {
                Err(api_disabled("process"))
            }
        },
        "screenshot" => {
            #[cfg(feature = "api-screenshot")]
            {
                screenshot::dispatch(method, args)
            }
            #[cfg(not(feature = "api-screenshot"))]
            {
                Err(api_disabled("screenshot"))
            }
        },
        "shell" => {
            #[cfg(feature = "api-shell")]
            {
                shell::dispatch(method, args)
            }
            #[cfg(not(feature = "api-shell"))]
            {
                Err(api_disabled("shell"))
            }
        },
        "shortcuts" => {
            #[cfg(feature = "api-shortcuts")]
            {
                shortcuts::dispatch(method, args)
            }
            #[cfg(not(feature = "api-shortcuts"))]
            {
                Err(api_disabled("shortcuts"))
            }
        },
        "sidecar" => {
            #[cfg(feature = "api-sidecar")]
            {
                sidecar::dispatch(method, args)
            }
            #[cfg(not(feature = "api-sidecar"))]
            {
                Err(api_disabled("sidecar"))
            }
        },
        "singleInstance" => {
            #[cfg(feature = "api-single-instance")]
            {
                single_instance::dispatch(method, args)
            }
            #[cfg(not(feature = "api-single-instance"))]
            {
                Err(api_disabled("singleInstance"))
            }
        },
        "store" => {
            #[cfg(feature = "api-store")]
            {
                store::dispatch(method, args)
            }
            #[cfg(not(feature = "api-store"))]
            {
                Err(api_disabled("store"))
            }
        },
        "systemStats" => {
            #[cfg(feature = "api-system-stats")]
            {
                system_stats::dispatch(method, args)
            }
            #[cfg(not(feature = "api-system-stats"))]
            {
                Err(api_disabled("system-stats"))
            }
        },
        "theme" => {
            #[cfg(feature = "api-theme")]
            {
                theme::dispatch(method, args)
            }
            #[cfg(not(feature = "api-theme"))]
            {
                Err(api_disabled("theme"))
            }
        },
        "updater" => {
            #[cfg(feature = "api-updater")]
            {
                updater::dispatch(method, args)
            }
            #[cfg(not(feature = "api-updater"))]
            {
                Err(api_disabled("updater"))
            }
        },
        "vault" => {
            #[cfg(feature = "api-vault")]
            {
                vault::dispatch(method, args)
            }
            #[cfg(not(feature = "api-vault"))]
            {
                Err(api_disabled("vault"))
            }
        },
        "websocket" => {
            #[cfg(feature = "api-websocket")]
            {
                websocket::dispatch(method, args)
            }
            #[cfg(not(feature = "api-websocket"))]
            {
                Err(api_disabled("websocket"))
            }
        },

        #[cfg(feature = "embedded-js")]
        "workers" => workers::dispatch(method, args),
        #[cfg(not(feature = "embedded-js"))]
        "workers" => {
            Err("workers API is lite-only; in full runtime use Bun.Worker natively".to_string())
        },

        _ => Err(format!("Unknown OS API: '{api}'")),
    }
}
