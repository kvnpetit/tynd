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
    // No std API; shell out to `hostname` — present on every target OS.
    match std::process::Command::new("hostname").output() {
        Ok(o) if o.status.success() => {
            Value::String(String::from_utf8_lossy(&o.stdout).trim().to_string())
        },
        _ => Value::Null,
    }
}

/// BCP-47 locale guess from common env vars (LC_ALL, LC_MESSAGES, LANG).
/// Good enough for UX defaults; apps needing strict BCP-47 should pass their
/// own parser over the result.
fn locale() -> Value {
    for var in ["LC_ALL", "LC_MESSAGES", "LANG", "LANGUAGE"] {
        if let Ok(v) = std::env::var(var) {
            let tag = v.split('.').next().unwrap_or(&v).replace('_', "-");
            if !tag.is_empty() && tag != "C" && tag != "POSIX" {
                return Value::String(tag);
            }
        }
    }
    Value::Null
}

fn eol() -> &'static str {
    if cfg!(windows) {
        "\r\n"
    } else {
        "\n"
    }
}

/// Read the current system color scheme. Falls back to `false` on platforms
/// where wry hasn't wired the detection (older WebKitGTK in particular).
fn is_dark_mode() -> bool {
    #[cfg(target_os = "windows")]
    {
        use std::process::Command;
        // Registry: 0 = dark, 1 = light. Absent -> light.
        let out = Command::new("reg")
            .args([
                "query",
                r"HKCU\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize",
                "/v",
                "AppsUseLightTheme",
            ])
            .output()
            .ok();
        if let Some(o) = out {
            if o.status.success() {
                let txt = String::from_utf8_lossy(&o.stdout);
                if let Some(idx) = txt.find("0x") {
                    let hex = &txt[idx + 2..idx + 3];
                    return hex == "0";
                }
            }
        }
        false
    }
    #[cfg(target_os = "macos")]
    {
        use std::process::Command;
        // `defaults read -g AppleInterfaceStyle` prints "Dark" when set, errors otherwise.
        let out = Command::new("defaults")
            .args(["read", "-g", "AppleInterfaceStyle"])
            .output()
            .ok();
        matches!(out, Some(o) if String::from_utf8_lossy(&o.stdout).trim().eq_ignore_ascii_case("dark"))
    }
    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    {
        use std::process::Command;
        // GNOME-ish desktops expose this via gsettings. Not universal but the common case.
        let out = Command::new("gsettings")
            .args(["get", "org.gnome.desktop.interface", "color-scheme"])
            .output()
            .ok();
        matches!(out, Some(o) if String::from_utf8_lossy(&o.stdout).to_lowercase().contains("dark"))
    }
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

/// Short OS version string (e.g. "10.0.26200" on Windows, "14.5" on macOS).
/// Best-effort via `ver` / `sw_vers` / `uname -r`.
fn os_version() -> Value {
    #[cfg(target_os = "windows")]
    {
        let out = std::process::Command::new("cmd")
            .args(["/c", "ver"])
            .output()
            .ok();
        if let Some(o) = out {
            if let Some(start) = String::from_utf8_lossy(&o.stdout).find('[') {
                let s = String::from_utf8_lossy(&o.stdout).to_string();
                if let Some(end) = s[start..].find(']') {
                    let segment = &s[start + 1..start + end];
                    // "Version 10.0.26200.xxx" -> "10.0.26200.xxx"
                    if let Some(rest) = segment.strip_prefix("Version ") {
                        return Value::String(rest.to_string());
                    }
                    return Value::String(segment.to_string());
                }
            }
        }
        Value::Null
    }
    #[cfg(target_os = "macos")]
    {
        let out = std::process::Command::new("sw_vers")
            .arg("-productVersion")
            .output()
            .ok();
        match out {
            Some(o) if o.status.success() => {
                Value::String(String::from_utf8_lossy(&o.stdout).trim().to_string())
            },
            _ => Value::Null,
        }
    }
    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    {
        let out = std::process::Command::new("uname").arg("-r").output().ok();
        match out {
            Some(o) if o.status.success() => {
                Value::String(String::from_utf8_lossy(&o.stdout).trim().to_string())
            },
            _ => Value::Null,
        }
    }
}
