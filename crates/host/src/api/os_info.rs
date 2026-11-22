//! Cross-platform OS metadata + standard directories.
//!
//! Backed by `os_info`, `hostname`, `sys-locale`, and `dirs`.

use serde_json::{json, Value};

use crate::api::HostApi;
use crate::tynd;

/// Stateless dispatch marker.
#[derive(Debug, Default)]
pub(crate) struct OsInfoApi;

/// Row of the host API table (`api::HOST_APIS`).
pub(crate) const API: HostApi = HostApi::new("os", |method, args| dispatch(method, &args));

#[tynd::handler(api = "os")]
impl OsInfoApi {
    /// Snapshot the host platform: `{ platform, arch, family, version }`.
    pub fn info() -> Value {
        info_impl()
    }

    /// Hostname via `hostname::get` (`gethostname(2)` / `GetComputerNameExW`).
    pub fn hostname() -> Value {
        hostname_impl()
    }

    /// Active BCP-47 locale via `sys-locale` — covers
    /// `GetUserDefaultLocaleName` (Win), `[NSLocale currentLocale]`
    /// (macOS), and `LC_*` env vars (Linux).
    pub fn locale() -> Value {
        locale_impl()
    }

    /// Snapshot the system color scheme as a single boolean — full
    /// `light` / `dark` / `unspecified` typing lives in `theme.rs`.
    pub fn is_dark_mode() -> bool {
        is_dark_mode_impl()
    }

    /// Platform-native end-of-line marker (`"\r\n"` on Windows, `"\n"`
    /// elsewhere). Returned as a string so the caller can splice it
    /// into text without re-deciding which platform they're on.
    pub fn eol() -> String {
        eol_str().to_string()
    }

    /// User home directory via `dirs::home_dir`. `null` on platforms
    /// or accounts where the OS doesn't surface one.
    pub fn home_dir() -> Value {
        path_or_null(dirs::home_dir())
    }

    /// Process-wide temp directory via `std::env::temp_dir`.
    pub fn tmp_dir() -> Value {
        path_to_value(&std::env::temp_dir())
    }

    /// XDG / Known Folder config directory. `null` on rare unconfigured hosts.
    pub fn config_dir() -> Value {
        path_or_null(dirs::config_dir())
    }

    /// XDG / Known Folder data directory.
    pub fn data_dir() -> Value {
        path_or_null(dirs::data_dir())
    }

    /// XDG / Known Folder cache directory.
    pub fn cache_dir() -> Value {
        path_or_null(dirs::cache_dir())
    }

    /// User desktop directory.
    pub fn desktop_dir() -> Value {
        path_or_null(dirs::desktop_dir())
    }

    /// User downloads directory.
    pub fn downloads_dir() -> Value {
        path_or_null(dirs::download_dir())
    }

    /// User documents directory.
    pub fn documents_dir() -> Value {
        path_or_null(dirs::document_dir())
    }

    /// User pictures directory.
    pub fn pictures_dir() -> Value {
        path_or_null(dirs::picture_dir())
    }

    /// User music / audio directory.
    pub fn music_dir() -> Value {
        path_or_null(dirs::audio_dir())
    }

    /// User video directory.
    pub fn video_dir() -> Value {
        path_or_null(dirs::video_dir())
    }

    /// Path of the currently-running executable.
    pub fn exe_path() -> Value {
        std::env::current_exe()
            .ok()
            .map_or(Value::Null, |p| path_to_value(&p))
    }

    /// Resource directory — the directory containing the running
    /// executable. Apps stash bundled assets next to the binary.
    pub fn resource_dir() -> Value {
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(std::path::Path::to_path_buf))
            .map_or(Value::Null, |p| path_to_value(&p))
    }

    /// Process working directory at the time of the call.
    pub fn cwd() -> Value {
        std::env::current_dir()
            .ok()
            .map_or(Value::Null, |p| path_to_value(&p))
    }

    /// Read environment variable `key`. Returns `null` when unset —
    /// matches the `process.env[k]` shape JS callers already expect.
    pub fn env(key: String) -> Value {
        std::env::var(&key).map_or(Value::Null, Value::String)
    }
}

fn hostname_impl() -> Value {
    // `hostname::get` wraps `gethostname(2)` (Unix) and
    // `GetComputerNameExW` (Windows) — drops the shell-out and the
    // `OsString::from_utf8_lossy` round-trip.
    hostname::get().ok().map_or(Value::Null, |h| {
        Value::String(h.to_string_lossy().into_owned())
    })
}

/// BCP-47 locale via `sys-locale`, which calls
/// `GetUserDefaultLocaleName` (Win), `[NSLocale currentLocale]` (macOS),
/// and `LC_*` env vars (Linux). Fixes a class of locale-detection bugs
/// the env-only fallback missed (Windows installs without the LC_ vars
/// set, modern macOS that exposes the locale via CoreFoundation).
fn locale_impl() -> Value {
    sys_locale::get_locale().map_or(Value::Null, |s| Value::String(s.replace('_', "-")))
}

fn eol_str() -> &'static str {
    if cfg!(windows) {
        "\r\n"
    } else {
        "\n"
    }
}

/// Read the current system color scheme via `dark-light`, which
/// queries `UISettings::GetColorValue(Foreground)` (Windows),
/// `NSUserDefaults` (macOS), and the freedesktop XDG portal D-Bus
/// `Settings.Read('color-scheme')` (Linux) — the latter covers KDE,
/// sway, and Flatpak'd apps. Without a portal, falls back to KDE
/// `kdeglobals` / GTK settings.
fn is_dark_mode_impl() -> bool {
    matches!(
        crate::platform::color_scheme::detect(),
        dark_light::Mode::Dark
    )
}

fn path_or_null(p: Option<std::path::PathBuf>) -> Value {
    p.map_or(Value::Null, |p| path_to_value(&p))
}

fn path_to_value(p: &std::path::Path) -> Value {
    Value::String(p.to_string_lossy().into_owned())
}

fn info_impl() -> Value {
    json!({
        "platform": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "family": std::env::consts::FAMILY,
        "version": os_version(),
    })
}

/// Short OS version string (e.g. `"10.0.26200"`, `"14.5"`).
///
/// `os_info::get()` queries the appropriate native API on each
/// platform: WMI / Registry on Windows, `sw_vers` (cached at
/// process-init) on macOS, and `/etc/os-release` + `uname` on Linux,
/// so the result does not depend on the Windows display language.
fn os_version() -> Value {
    let info = os_info::get();
    match info.version() {
        os_info::Version::Unknown => Value::Null,
        v => Value::String(v.to_string()),
    }
}
