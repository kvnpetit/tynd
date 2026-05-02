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

#![cfg(target_os = "windows")]

use std::io;

use winreg::{enums::HKEY_CURRENT_USER, RegKey};

fn classes_path(scheme: &str) -> String {
    format!("Software\\Classes\\{scheme}")
}

fn open_command(exe: &str) -> String {
    format!("\"{exe}\" \"%1\"")
}

pub(super) fn register(scheme: &str, exe: &str) -> io::Result<()> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (root, _) = hkcu.create_subkey(classes_path(scheme))?;
    root.set_value("", &format!("URL:{scheme} Protocol"))?;
    root.set_value("URL Protocol", &"")?;

    let (cmd, _) = hkcu.create_subkey(format!("{}\\shell\\open\\command", classes_path(scheme)))?;
    cmd.set_value("", &open_command(exe))?;
    Ok(())
}

pub(super) fn unregister(scheme: &str) -> io::Result<()> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    match hkcu.delete_subkey_all(classes_path(scheme)) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

pub(super) fn is_registered(scheme: &str, exe: &str) -> io::Result<bool> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let cmd = match hkcu.open_subkey(format!("{}\\shell\\open\\command", classes_path(scheme))) {
        Ok(k) => k,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e),
    };
    let cur: String = match cmd.get_value("") {
        Ok(v) => v,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(false),
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
