//! App-level APIs: name / version / exit / relaunch.
//!
//! Backend sets name + version via `app.setInfo`; frontend reads via
//! `app.getName` / `app.getVersion`.

use std::{path::Path, sync::LazyLock, time::Duration};

use parking_lot::Mutex;
use serde_json::Value;

use crate::api::HostApi;
use crate::ipc::error::IpcError;
use crate::security::policy;
use crate::tynd;

mod dock;
mod jump_list;
mod recent_documents;

// Empty string = "backend hasn't called `app.setInfo` yet" — getters fall
// back to the process-derived default.
static APP_NAME: LazyLock<Mutex<String>> = LazyLock::new(|| Mutex::new(String::new()));
static APP_VERSION: LazyLock<Mutex<String>> = LazyLock::new(|| Mutex::new(String::new()));
static BUNDLE_ID: LazyLock<Mutex<String>> = LazyLock::new(|| Mutex::new(String::new()));

/// Tynd host version — baked in at compile time from the workspace Cargo.toml.
const FRAMEWORK_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Dispatch marker — state lives in `APP_NAME` / `APP_VERSION` / `BUNDLE_ID`.
#[derive(Debug, Default)]
pub(crate) struct AppApi;

/// Row of the host API table (`api::HOST_APIS`).
pub(crate) const API: HostApi = HostApi::new("app", |method, args| dispatch(method, &args));

#[tynd::handler(api = "app")]
impl AppApi {
    /// Return the app's display name. Defaults to the running
    /// binary's filename when the backend hasn't called `setInfo`.
    pub fn get_name() -> String {
        get_name()
    }

    /// Return the app version string. Defaults to `"0.0.0"` when the
    /// backend hasn't called `setInfo`.
    pub fn get_version() -> String {
        get_version()
    }

    /// Return the Tynd framework version baked into this host.
    pub fn get_framework_version() -> String {
        FRAMEWORK_VERSION.to_string()
    }

    /// Return the app's bundle identifier (reverse-DNS string), or
    /// `null` when the backend hasn't called `setInfo`.
    pub fn get_bundle_identifier() -> Option<String> {
        get_bundle_id()
    }

    /// Set the app's display metadata. Each field is optional —
    /// supply only the keys you want to update.
    pub fn set_info(name: Option<String>, version: Option<String>, bundle_id: Option<String>) {
        if let Some(n) = name {
            *APP_NAME.lock() = n;
        }
        if let Some(v) = version {
            *APP_VERSION.lock() = v;
        }
        if let Some(b) = bundle_id {
            *BUNDLE_ID.lock() = b;
        }
    }

    /// Switch `app:open-file` to live delivery and return the paths opened
    /// before the first subscription (`{ paths: string[] }`).
    pub fn subscribe_open_files() -> Value {
        crate::app::subscribe_open_files()
    }

    /// Add a file to the OS recent-documents list: the app's jump list
    /// "Recent" category on Windows (shown once the app is registered for
    /// the file type), the "Open Recent" list on macOS. Rejected on Linux.
    /// The path must pass the FS scope and name an existing file.
    pub fn add_recent_document(path: String) -> Result<(), String> {
        let app_err = |e: String| IpcError::new("app", "addRecentDocument", e).into_string();
        let resolved = policy::authorize_fs_path(&path).map_err(app_err)?;
        if !resolved.is_file() {
            return Err(app_err(format!("'{path}' is not an existing file")));
        }
        recent_documents::add(&resolved).map_err(app_err)
    }

    /// Clear the app's recent-documents list (Windows jump list entries,
    /// macOS "Open Recent"). Rejected on Linux.
    pub fn clear_recent_documents() -> Result<(), String> {
        recent_documents::clear()
            .map_err(|e| IpcError::new("app", "clearRecentDocuments", e).into_string())
    }

    /// macOS activation policy: `"regular"` (Dock icon + menu bar),
    /// `"accessory"` (no Dock icon, for menu-bar apps) or `"prohibited"`.
    /// Rejected on Windows and Linux.
    pub fn set_activation_policy(policy: String) -> Result<(), String> {
        let policy = dock::parse_policy("setActivationPolicy", &policy)?;
        dock::set_policy(policy)
            .map_err(|e| IpcError::new("app", "setActivationPolicy", e).into_string())
    }

    /// Bounce the macOS Dock icon: `"informational"` (once, default) or
    /// `"critical"` (until the app is activated). Returns the request id for
    /// `dockCancelBounce` (`0` when the app is already active). Rejected on
    /// Windows and Linux (use `window.requestAttention`).
    pub fn dock_bounce(kind: Option<String>) -> Result<i64, String> {
        let critical = dock::parse_bounce(kind.as_deref())?;
        dock::bounce(critical).map_err(|e| IpcError::new("app", "dockBounce", e).into_string())
    }

    /// Stop a bounce started by `dockBounce`. Unknown ids are ignored.
    pub fn dock_cancel_bounce(id: i64) -> Result<(), String> {
        dock::cancel_bounce(id)
            .map_err(|e| IpcError::new("app", "dockCancelBounce", e).into_string())
    }

    /// Replace the macOS Dock icon with the image at `path` (FS scope
    /// applies); `null` restores the bundle icon. Rejected on Windows and
    /// Linux (use `window.setWindowIcon`).
    pub fn set_dock_icon(path: Option<String>) -> Result<(), String> {
        let app_err = |e: String| IpcError::new("app", "setDockIcon", e).into_string();
        let resolved = match path {
            Some(path) => {
                let resolved = policy::authorize_fs_path(&path).map_err(app_err)?;
                if !resolved.is_file() {
                    return Err(app_err(format!("'{path}' is not an existing file")));
                }
                Some(resolved)
            },
            None => None,
        };
        dock::set_icon(resolved).map_err(app_err)
    }

    /// Set the menu shown when the user right-clicks the macOS Dock icon
    /// (same item shape as the application menu; clicks emit
    /// `menu:action`). An empty array removes it. Rejected on Windows and
    /// Linux.
    /// Requires the `menu` API in the build.
    pub fn set_dock_menu(items: Value) -> Result<(), String> {
        let items = dock::parse_menu(items)?;
        dock::set_menu(items).map_err(|e| IpcError::new("app", "setDockMenu", e).into_string())
    }

    /// Replace the Windows jump list: `tasks` (links in the "Tasks" section)
    /// and `categories` (named sections), each item relaunching the app
    /// executable with its `args` (forwarded to the running instance by the
    /// single-instance API). `showRecent` (default `true`) keeps the OS
    /// "Recent" category. Icon paths go through the FS scope. Rejected on
    /// macOS (use `setDockMenu`) and Linux.
    pub fn set_jump_list(
        tasks: Option<Value>,
        categories: Option<Value>,
        show_recent: Option<bool>,
    ) -> Result<(), String> {
        let app_err = |e: String| IpcError::new("app", "setJumpList", e).into_string();
        let authorize_icon = |path: &str| {
            let resolved = policy::authorize_fs_path(path)?;
            if resolved.is_file() {
                Ok(resolved)
            } else {
                Err(format!("'{path}' is not an existing file"))
            }
        };
        let list =
            jump_list::parse(tasks, categories, show_recent, &authorize_icon).map_err(&app_err)?;
        jump_list::apply(&list).map_err(app_err)
    }

    /// Remove the app's custom Windows jump list (tasks and categories).
    /// Rejected on macOS and Linux.
    pub fn clear_jump_list() -> Result<(), String> {
        jump_list::clear().map_err(|e| IpcError::new("app", "clearJumpList", e).into_string())
    }

    /// Exit the host process with `code` (default `0`). Runs the
    /// cleanup hook chain before terminating. Never returns.
    pub fn exit(code: Option<i64>) {
        crate::cleanup::run();
        std::process::exit(code.unwrap_or(0) as i32);
    }

    /// Spawn a fresh copy of the running binary with the same arguments,
    /// then exit the current process. Used by self-update flows that want a
    /// clean state at re-launch.
    pub fn relaunch() -> Result<Value, String> {
        let exe = std::env::current_exe().map_err(|e| {
            IpcError::new("app", "relaunch", format!("current_exe: {e}")).into_string()
        })?;
        relaunch_and_exit(&exe)
            .map_err(|error| IpcError::new("app", "relaunch", error).into_string())?;
        Ok(Value::Null)
    }
}

/// Start `exe` with this process's arguments (minus deep-link URLs, see
/// `relaunch_args`), then schedule this process's exit. The single-instance and deep-link master locks are released first:
/// the old process only drops them at the end of cleanup, which can outlast
/// the new process's startup and make it forward to the dying primary and
/// quit. On failure the locks are re-acquired and nothing is scheduled.
pub(crate) fn relaunch_and_exit(exe: &Path) -> Result<(), String> {
    let released = RelaunchLocks::release();
    let mut child = match std::process::Command::new(exe)
        .args(relaunch_args())
        .spawn()
    {
        Ok(child) => child,
        Err(error) => {
            released.restore();
            return Err(format!("spawn: {error}"));
        },
    };
    if let Err(error) = schedule_exit(Duration::from_millis(100), 0) {
        let _ = child.kill();
        let _ = child.wait();
        released.restore();
        return Err(format!("exit scheduler: {error}"));
    }
    Ok(())
}

/// Arguments forwarded to a relaunched process: `argv[1..]`, minus the
/// deep-link URLs the new process would otherwise handle a second time.
pub(crate) fn relaunch_args() -> Vec<std::ffi::OsString> {
    #[cfg(feature = "api-deep-link")]
    {
        super::deep_link::relaunch_args()
    }
    #[cfg(not(feature = "api-deep-link"))]
    {
        std::env::args_os().skip(1).collect()
    }
}

/// Instance locks handed over to the relaunched process.
struct RelaunchLocks {
    #[cfg(feature = "api-single-instance")]
    single_instance: Option<String>,
    #[cfg(feature = "api-deep-link")]
    deep_link: bool,
}

impl RelaunchLocks {
    fn release() -> Self {
        Self {
            #[cfg(feature = "api-single-instance")]
            single_instance: super::single_instance::release_for_relaunch(),
            #[cfg(feature = "api-deep-link")]
            deep_link: super::deep_link::release_for_relaunch(),
        }
    }

    #[cfg_attr(
        not(any(feature = "api-single-instance", feature = "api-deep-link")),
        allow(
            clippy::unused_self,
            reason = "no lock to restore without these features"
        )
    )]
    fn restore(&self) {
        #[cfg(feature = "api-single-instance")]
        if let Some(id) = &self.single_instance {
            super::single_instance::restore_after_failed_relaunch(id);
        }
        #[cfg(feature = "api-deep-link")]
        if self.deep_link {
            super::deep_link::restore_after_failed_relaunch();
        }
    }
}

pub(crate) fn schedule_exit(delay: Duration, code: i32) -> std::io::Result<()> {
    std::thread::Builder::new()
        .name("tynd-delayed-exit".into())
        .spawn(move || {
            std::thread::sleep(delay);
            crate::cleanup::run();
            std::process::exit(code);
        })
        .map(|_| ())
}

/// Value of a `setInfo` field, or `None` while the backend hasn't set it.
fn configured(field: &Mutex<String>) -> Option<String> {
    let value = field.lock();
    (!value.is_empty()).then(|| value.clone())
}

fn get_bundle_id() -> Option<String> {
    configured(&BUNDLE_ID)
}

fn get_name() -> String {
    // Fallback: filename of the running binary without extension.
    configured(&APP_NAME).unwrap_or_else(|| {
        std::env::current_exe()
            .ok()
            .and_then(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
            .unwrap_or_else(|| "tynd".into())
    })
}

/// Version the backend declared through `setInfo`, or `None` when it never
/// did. The updater's downgrade floor: unlike [`get_version`] it does not
/// invent `"0.0.0"`, which would accept every manifest as newer.
#[cfg(feature = "api-updater")]
pub(crate) fn configured_version() -> Option<String> {
    configured(&APP_VERSION)
}

fn get_version() -> String {
    configured(&APP_VERSION).unwrap_or_else(|| "0.0.0".into())
}
