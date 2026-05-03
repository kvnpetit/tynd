//! Windows-side URL scheme registration via `HKCU\Software\Classes`.
//!
//! Per-user (no admin elevation needed) — the same hive Microsoft
//! documents for protocol activation. Layout we write:
//!
//! ```text
//! HKCU\Software\Classes\<scheme>\
//!     (Default) = "URL:<scheme> Protocol"
//!     URL Protocol = ""
//!     shell\
//!         open\
//!             command\
//!                 (Default) = "<exe>" "%1"
//! ```
//!
//! The string `"<exe>" "%1"` is exactly what the shell substitutes the
//! launched URL into; the surrounding quotes are essential when the
//! exe path contains spaces (which it always does on `Program Files`
//! installs).
//!
//! Backed by [`windows-registry`] — the typed wrapper from the
//! `microsoft/windows-rs` umbrella, which replaced the unmaintained
//! `winreg` crate in this module. The error type carries an `HRESULT`
//! we can match against `ERROR_FILE_NOT_FOUND` (`0x80070002`) for the
//! "absent key" branches.

#![cfg(target_os = "windows")]

use windows_registry::{Result, CURRENT_USER};

/// Win32 `ERROR_FILE_NOT_FOUND` mapped into HRESULT space
/// (`HRESULT_FROM_WIN32(2)` = `0x8007_0002`). `windows-registry`
/// surfaces missing keys / values through this code, mirroring what
/// `winreg` used to translate into `io::ErrorKind::NotFound`.
const HRESULT_NOT_FOUND: i32 = 0x8007_0002_u32 as i32;

fn classes_path(scheme: &str) -> String {
    format!("Software\\Classes\\{scheme}")
}

fn open_command(exe: &str) -> String {
    format!("\"{exe}\" \"%1\"")
}

pub(super) fn register(scheme: &str, exe: &str) -> Result<()> {
    let root = CURRENT_USER.create(classes_path(scheme))?;
    root.set_string("", &format!("URL:{scheme} Protocol"))?;
    root.set_string("URL Protocol", "")?;

    let cmd = CURRENT_USER.create(format!("{}\\shell\\open\\command", classes_path(scheme)))?;
    cmd.set_string("", open_command(exe))?;
    Ok(())
}

pub(super) fn unregister(scheme: &str) -> Result<()> {
    match CURRENT_USER.remove_tree(classes_path(scheme)) {
        Ok(()) => Ok(()),
        Err(e) if e.code().0 == HRESULT_NOT_FOUND => Ok(()),
        Err(e) => Err(e),
    }
}

pub(super) fn is_registered(scheme: &str, exe: &str) -> Result<bool> {
    let cmd_path = format!("{}\\shell\\open\\command", classes_path(scheme));
    let cmd = match CURRENT_USER.open(&cmd_path) {
        Ok(k) => k,
        Err(e) if e.code().0 == HRESULT_NOT_FOUND => return Ok(false),
        Err(e) => return Err(e),
    };
    let cur: String = match cmd.get_string("") {
        Ok(v) => v,
        Err(e) if e.code().0 == HRESULT_NOT_FOUND => return Ok(false),
        Err(e) => return Err(e),
    };
    let trimmed = cur.trim_start_matches('"');
    let end = trimmed.find('"').unwrap_or(trimmed.len());
    let registered_exe = &trimmed[..end];
    Ok(registered_exe.eq_ignore_ascii_case(exe))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_command_formats_quoted_exe() {
        assert_eq!(
            open_command("C:\\Program Files\\My App\\app.exe"),
            "\"C:\\Program Files\\My App\\app.exe\" \"%1\""
        );
    }

    #[test]
    fn classes_path_includes_scheme() {
        assert_eq!(classes_path("myapp"), "Software\\Classes\\myapp");
    }

    #[test]
    #[ignore = "writes to HKCU; opt in with `cargo test -- --ignored`"]
    fn register_round_trip_in_hkcu() {
        let scheme = format!("tynddltest{}", std::process::id());
        let exe = "C:\\fake\\app.exe";
        register(&scheme, exe).unwrap();
        assert!(is_registered(&scheme, exe).unwrap());
        assert!(!is_registered(&scheme, "C:\\other\\app.exe").unwrap());
        unregister(&scheme).unwrap();
        assert!(!is_registered(&scheme, exe).unwrap());
        unregister(&scheme).unwrap();
    }
}
