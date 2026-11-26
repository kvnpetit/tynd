//! System color-scheme detection shared by `os.info` and `os.theme`.

/// Detect the system color scheme. `dark-light` 3 only reads the freedesktop
/// portal on Linux; when that yields nothing (XFCE / i3 / older GNOME without
/// `xdg-desktop-portal`), fall back to KDE `kdeglobals` and the GTK settings,
/// like `dark-light` 1/2 did.
pub(crate) fn detect() -> dark_light::Mode {
    let mode = dark_light::detect().unwrap_or(dark_light::Mode::Unspecified);
    #[cfg(target_os = "linux")]
    {
        if matches!(mode, dark_light::Mode::Unspecified) {
            match linux_fallback::is_dark() {
                Some(true) => return dark_light::Mode::Dark,
                Some(false) => return dark_light::Mode::Light,
                None => {},
            }
        }
    }
    mode
}

/// Best-effort Linux detection for desktops without a portal. Never panics;
/// `None` means "no usable signal".
#[cfg(target_os = "linux")]
mod linux_fallback {
    use std::{
        process::{Command, Stdio},
        sync::Mutex,
        time::{Duration, Instant},
    };

    /// The theme watcher polls every second; re-reading the files and spawning
    /// `gsettings` at most every 5 s keeps that cheap.
    const CACHE_TTL: Duration = Duration::from_secs(5);
    static CACHE: Mutex<Option<(Instant, Option<bool>)>> = Mutex::new(None);

    pub(super) fn is_dark() -> Option<bool> {
        let mut cache = CACHE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match *cache {
            Some((at, value)) if at.elapsed() < CACHE_TTL => return value,
            _ => {},
        }
        let value = kde_is_dark().or_else(gtk_is_dark);
        *cache = Some((Instant::now(), value));
        value
    }

    /// `[Colors:Window] BackgroundNormal=r,g,b` luminance, else a `ColorScheme`
    /// name containing "dark".
    fn kde_is_dark() -> Option<bool> {
        let path = dirs::config_dir()
            .map(|d| d.join("kdeglobals"))
            .filter(|p| p.exists())
            .unwrap_or_else(|| std::path::PathBuf::from("/etc/xdg/kdeglobals"));
        parse_kdeglobals(&std::fs::read_to_string(path).ok()?)
    }

    fn parse_kdeglobals(text: &str) -> Option<bool> {
        let mut section = "";
        let mut background = None;
        let mut scheme = None;
        for line in text.lines().map(str::trim) {
            if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
                section = name;
            } else if let Some((key, value)) = line.split_once('=') {
                match (section, key.trim()) {
                    ("Colors:Window", "BackgroundNormal") => background = Some(value.trim()),
                    ("General", "ColorScheme") => scheme = Some(value.trim()),
                    _ => {},
                }
            }
        }
        if let Some(value) = background {
            let rgb: Vec<f64> = value
                .split(',')
                .map(|c| c.trim().parse::<f64>())
                .collect::<Result<_, _>>()
                .ok()?;
            if rgb.len() >= 3 {
                let luma = 0.299 * rgb[0] + 0.587 * rgb[1] + 0.114 * rgb[2];
                return Some(luma < 128.0);
            }
        }
        scheme.map(|s| s.to_lowercase().contains("dark"))
    }

    fn gsettings(key: &str) -> Option<String> {
        let out = Command::new("gsettings")
            .args(["get", "org.gnome.desktop.interface", key])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let value = String::from_utf8_lossy(&out.stdout)
            .trim()
            .trim_matches('\'')
            .to_lowercase();
        (!value.is_empty()).then_some(value)
    }

    fn gtk_is_dark() -> Option<bool> {
        match gsettings("color-scheme").as_deref() {
            Some("prefer-dark") => return Some(true),
            Some("prefer-light") => return Some(false),
            _ => {},
        }
        gsettings("gtk-theme").map(|theme| theme.contains("dark"))
    }

    #[cfg(test)]
    mod tests {
        use super::parse_kdeglobals;

        #[test]
        fn kdeglobals_background_luminance() {
            let dark = "[Colors:Window]\nBackgroundNormal=35,38,41\n";
            let light = "[Colors:Window]\nBackgroundNormal=239,240,241\n";
            assert_eq!(parse_kdeglobals(dark), Some(true));
            assert_eq!(parse_kdeglobals(light), Some(false));
            assert_eq!(
                parse_kdeglobals("[General]\nColorScheme=BreezeDark\n"),
                Some(true)
            );
            assert_eq!(parse_kdeglobals("[Other]\nx=1\n"), None);
        }
    }
}
