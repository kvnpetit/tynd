//! Regional settings: region, preferred languages, time zone and clock
//! format.

/// ISO 3166 region (or UN M.49 area code) the user picked in the OS
/// settings: `GetUserDefaultGeoName` on Windows, `NSLocale.regionCode` on
/// macOS, the region of the locale on Linux.
pub(super) fn region() -> Option<String> {
    imp::region()
}

/// Preferred UI languages, most preferred first (BCP-47 tags).
pub(super) fn languages() -> Vec<String> {
    let mut seen = Vec::new();
    for tag in sys_locale::get_locales().map(|tag| tag.replace('_', "-")) {
        let tag = strip_posix_suffix(&tag);
        if !tag.is_empty() && tag != "C" && tag != "POSIX" && !seen.iter().any(|s| s == tag) {
            seen.push(tag.to_string());
        }
    }
    seen
}

/// IANA time zone name (`"Europe/Paris"`), `None` when the OS reports none.
pub(super) fn timezone() -> Option<String> {
    iana_time_zone::get_timezone().ok()
}

/// `true` when the user's time format is 24-hour, `None` when unknown.
pub(super) fn uses_24_hour_clock() -> Option<bool> {
    imp::uses_24_hour_clock()
}

/// `"fr-FR.UTF-8"` / `"de-DE@euro"` → `"fr-FR"` / `"de-DE"`.
fn strip_posix_suffix(tag: &str) -> &str {
    tag.split(['.', '@']).next().unwrap_or(tag)
}

/// Region subtag of a locale tag: `"fr-FR"` → `FR`, `"zh-Hans-CN"` → `CN`,
/// `"es-419"` → `419`; `None` without one.
#[cfg_attr(not(any(target_os = "linux", test)), allow(dead_code))]
fn region_of_locale(tag: &str) -> Option<String> {
    let tag = strip_posix_suffix(tag);
    tag.split(['-', '_'])
        .skip(1)
        .find(|part| {
            (part.len() == 2 && part.chars().all(|c| c.is_ascii_alphabetic()))
                || (part.len() == 3 && part.chars().all(|c| c.is_ascii_digit()))
        })
        .map(str::to_ascii_uppercase)
}

/// Clock style of a Windows / ICU (CLDR) time pattern: `H` / `k` mean
/// 24-hour, `h` / `K` 12-hour. Text between single quotes is literal.
#[cfg_attr(
    not(any(target_os = "windows", target_os = "macos", test)),
    allow(dead_code)
)]
fn pattern_uses_24_hour(pattern: &str) -> Option<bool> {
    let mut quoted = false;
    for c in pattern.chars() {
        match c {
            '\'' => quoted = !quoted,
            'H' | 'k' if !quoted => return Some(true),
            'h' | 'K' if !quoted => return Some(false),
            _ => {},
        }
    }
    None
}

/// Clock style of a POSIX `strftime` time format (`nl_langinfo(T_FMT)`).
#[cfg_attr(not(any(target_os = "linux", test)), allow(dead_code))]
fn posix_format_uses_24_hour(format: &str) -> Option<bool> {
    let mut chars = format.chars();
    while let Some(c) = chars.next() {
        if c != '%' {
            continue;
        }
        match chars.next() {
            Some('H' | 'k' | 'R' | 'T') => return Some(true),
            Some('I' | 'l' | 'p' | 'P' | 'r') => return Some(false),
            _ => {},
        }
    }
    None
}

#[cfg(target_os = "windows")]
mod imp {
    use windows_sys::Win32::Globalization::{
        GetLocaleInfoEx, GetUserDefaultGeoName, LOCALE_STIMEFORMAT,
    };

    /// Longest locale string we read (`LOCALE_NAME_MAX_LENGTH` is 85).
    const BUFFER: usize = 128;

    fn read(fill: impl FnOnce(*mut u16, i32) -> i32) -> Option<String> {
        let mut buffer = [0u16; BUFFER];
        let written = fill(buffer.as_mut_ptr(), i32::try_from(BUFFER).ok()?);
        let len = usize::try_from(written).ok()?.checked_sub(1)?;
        let text = String::from_utf16_lossy(buffer.get(..len)?);
        (!text.is_empty()).then_some(text)
    }

    pub(super) fn region() -> Option<String> {
        // SAFETY: the buffer and its length in UTF-16 units come from `read`.
        read(|buffer, len| unsafe { GetUserDefaultGeoName(buffer, len) })
    }

    pub(super) fn uses_24_hour_clock() -> Option<bool> {
        // SAFETY: a null locale name selects the user default locale; the
        // buffer and its length come from `read`.
        let pattern = read(|buffer, len| unsafe {
            GetLocaleInfoEx(std::ptr::null(), LOCALE_STIMEFORMAT, buffer, len)
        })?;
        super::pattern_uses_24_hour(&pattern)
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use objc2_foundation::{ns_string, NSDateFormatter, NSLocale};

    pub(super) fn region() -> Option<String> {
        NSLocale::currentLocale()
            .regionCode()
            .map(|code| code.to_string())
    }

    pub(super) fn uses_24_hour_clock() -> Option<bool> {
        let locale = NSLocale::currentLocale();
        // `j` is the CLDR "preferred hour" skeleton: it expands to `H` or
        // `h a` following the user's 12/24-hour setting.
        let pattern = NSDateFormatter::dateFormatFromTemplate_options_locale(
            ns_string!("j"),
            0,
            Some(&locale),
        )?;
        super::pattern_uses_24_hour(&pattern.to_string())
    }
}

#[cfg(target_os = "linux")]
mod imp {
    pub(super) fn region() -> Option<String> {
        ["LC_ALL", "LC_TIME", "LC_MESSAGES", "LANG"]
            .iter()
            .filter_map(|key| std::env::var(key).ok())
            .find(|value| !value.is_empty())
            .and_then(|value| super::region_of_locale(&value))
            .or_else(|| sys_locale::get_locale().and_then(|tag| super::region_of_locale(&tag)))
    }

    pub(super) fn uses_24_hour_clock() -> Option<bool> {
        // SAFETY: `nl_langinfo` returns a pointer to a static NUL-terminated
        // string of the current LC_TIME locale (set from the environment by
        // GTK at startup), copied out immediately.
        let format = unsafe {
            let raw = libc::nl_langinfo(libc::T_FMT);
            if raw.is_null() {
                return None;
            }
            std::ffi::CStr::from_ptr(raw).to_string_lossy().into_owned()
        };
        super::posix_format_uses_24_hour(&format)
    }
}

#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
mod imp {
    pub(super) fn region() -> Option<String> {
        sys_locale::get_locale().and_then(|tag| super::region_of_locale(&tag))
    }

    pub(super) fn uses_24_hour_clock() -> Option<bool> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn region_subtags() {
        assert_eq!(region_of_locale("fr-FR").as_deref(), Some("FR"));
        assert_eq!(region_of_locale("en_us.UTF-8").as_deref(), Some("US"));
        assert_eq!(region_of_locale("zh-Hans-CN").as_deref(), Some("CN"));
        assert_eq!(region_of_locale("es-419").as_deref(), Some("419"));
        assert_eq!(region_of_locale("de_DE@euro").as_deref(), Some("DE"));
        assert_eq!(region_of_locale("C"), None);
        assert_eq!(region_of_locale("en"), None);
    }

    #[test]
    fn cldr_and_windows_patterns() {
        assert_eq!(pattern_uses_24_hour("HH:mm:ss"), Some(true));
        assert_eq!(pattern_uses_24_hour("h:mm:ss tt"), Some(false));
        assert_eq!(pattern_uses_24_hour("h a"), Some(false));
        assert_eq!(pattern_uses_24_hour("'h'H"), Some(true));
        assert_eq!(pattern_uses_24_hour("K:mm a"), Some(false));
        assert_eq!(pattern_uses_24_hour("mm:ss"), None);
    }

    #[test]
    fn posix_formats() {
        assert_eq!(posix_format_uses_24_hour("%H:%M:%S"), Some(true));
        assert_eq!(posix_format_uses_24_hour("%T"), Some(true));
        assert_eq!(posix_format_uses_24_hour("%I:%M:%S %p"), Some(false));
        assert_eq!(posix_format_uses_24_hour("%r"), Some(false));
        assert_eq!(posix_format_uses_24_hour("%%H"), None);
        assert_eq!(posix_format_uses_24_hour(""), None);
    }

    #[test]
    fn posix_suffixes_are_stripped() {
        assert_eq!(strip_posix_suffix("fr-FR.UTF-8"), "fr-FR");
        assert_eq!(strip_posix_suffix("de-DE@euro"), "de-DE");
        assert_eq!(strip_posix_suffix("en-US"), "en-US");
    }
}
