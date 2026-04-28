use serde_json::{json, Value};

pub fn dispatch(method: &str, args: &Value) -> Result<Value, String> {
    match method {
        "info" => Ok(info()),
        "hostname" => Ok(hostname()),
        "locale" => Ok(locale()),
        "isDarkMode" => Ok(Value::Bool(is_dark_mode())),
        "eol" => Ok(Value::String(eol().into())),
        "homeDir" => Ok(path_or_null(dirs::home_dir())),
        "tmpDir" => Ok(path_to_value(&std::env::temp_dir())),
        "configDir" => Ok(path_or_null(dirs::config_dir())),
        "dataDir" => Ok(path_or_null(dirs::data_dir())),
        "cacheDir" => Ok(path_or_null(dirs::cache_dir())),
        "desktopDir" => Ok(path_or_null(dirs::desktop_dir())),
        "downloadsDir" => Ok(path_or_null(dirs::download_dir())),
        "documentsDir" => Ok(path_or_null(dirs::document_dir())),
        "picturesDir" => Ok(path_or_null(dirs::picture_dir())),
        "musicDir" => Ok(path_or_null(dirs::audio_dir())),
        "videoDir" => Ok(path_or_null(dirs::video_dir())),
        "exePath" => Ok(std::env::current_exe()
            .ok()
            .map_or(Value::Null, |p| path_to_value(&p))),
        "resourceDir" => Ok(std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(std::path::Path::to_path_buf))
            .map_or(Value::Null, |p| path_to_value(&p))),
        "cwd" => Ok(std::env::current_dir()
            .ok()
            .map_or(Value::Null, |p| path_to_value(&p))),
        "env" => {
            let key = args
                .get("key")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "os.env: missing 'key'".to_string())?;
            Ok(std::env::var(key).map_or(Value::Null, Value::String))
        },
        _ => Err(format!("os.{method}: unknown method")),
    }
}

fn hostname() -> Value {
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
fn locale() -> Value {
    sys_locale::get_locale().map_or(Value::Null, |s| Value::String(s.replace('_', "-")))
}

fn eol() -> &'static str {
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
/// sway, and Flatpak'd apps that the previous `gsettings` shell-out
/// missed entirely.
fn is_dark_mode() -> bool {
    matches!(dark_light::detect(), Ok(dark_light::Mode::Dark))
}

fn path_or_null(p: Option<std::path::PathBuf>) -> Value {
    p.map_or(Value::Null, |p| path_to_value(&p))
}

fn path_to_value(p: &std::path::Path) -> Value {
    Value::String(p.to_string_lossy().into_owned())
}

fn info() -> Value {
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
/// process-init) on macOS, and `/etc/os-release` + `uname` on Linux.
/// Replaces the previous `cmd /c ver` parser that broke on non-EN
/// Windows installs (the localised `Version ` prefix wouldn't strip).
fn os_version() -> Value {
    let info = os_info::get();
    match info.version() {
        os_info::Version::Unknown => Value::Null,
        v => Value::String(v.to_string()),
    }
}
