//! Linux-side URL scheme registration via XDG `.desktop` entries.
//!
//! Per the Desktop Entry Specification, a per-user entry living in
//! `$XDG_DATA_HOME/applications/<app_id>.desktop` (default
//! `~/.local/share/applications/`) becomes the OS-default handler for
//! `<scheme>://…` once `xdg-mime default <app_id>.desktop
//! x-scheme-handler/<scheme>` has been run. We do both — write the
//! file, then shell out to `xdg-mime`.
//!
//! The per-user `mimeapps.list` is the source of truth most desktop
//! environments consult; `is_registered` reads it back to confirm the
//! association we just wrote is still there.

#![cfg(target_os = "linux")]

use std::{
    fs,
    io::{self, BufRead, BufReader, Write},
    path::PathBuf,
    process::Command,
};

fn app_id() -> String {
    let exe = std::env::current_exe()
        .ok()
        .and_then(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "tynd-app".into());
    exe.replace(['/', '\\'], "-")
}

fn applications_dir() -> io::Result<PathBuf> {
    let base = if let Ok(xdg) = std::env::var("XDG_DATA_HOME") {
        PathBuf::from(xdg)
    } else {
        let home = dirs::home_dir()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "could not locate $HOME"))?;
        home.join(".local").join("share")
    };
    let dir = base.join("applications");
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

fn desktop_file_path() -> io::Result<PathBuf> {
    Ok(applications_dir()?.join(format!("{}.desktop", app_id())))
}

/// Quote `exe` for an `Exec=` value per the Desktop Entry spec:
/// double-quote the whole string and backslash-escape `\`, `"`, `` ` ``,
/// and `$` so a path containing spaces or shell-meaningful characters
/// can't reshape the launched argv.
fn quote_desktop_exec(exe: &str) -> String {
    let mut out = String::with_capacity(exe.len() + 2);
    out.push('"');
    for ch in exe.chars() {
        if matches!(ch, '\\' | '"' | '`' | '$') {
            out.push('\\');
        }
        out.push(ch);
    }
    out.push('"');
    out
}

/// Sanitize a Desktop Entry "string" value (per spec: no control chars).
/// Rejects NUL outright (would terminate the file mid-line on read);
/// strips `\r` and `\n` so an attacker-supplied value can't inject a
/// fresh `Exec=` / `Name=` key on a synthetic next line.
fn sanitize_desktop_value(s: &str) -> io::Result<String> {
    if s.contains('\0') {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "desktop entry value contains NUL byte",
        ));
    }
    Ok(s.chars().filter(|c| *c != '\r' && *c != '\n').collect())
}

pub(super) fn render_desktop_entry(name: &str, exe: &str, scheme: &str) -> io::Result<String> {
    let quoted = quote_desktop_exec(exe);
    let safe_name = sanitize_desktop_value(name)?;
    let safe_scheme = sanitize_desktop_value(scheme)?;
    Ok(format!(
        "[Desktop Entry]\nType=Application\nName={safe_name}\nExec={quoted} \
         %u\nTerminal=false\nNoDisplay=true\nMimeType=x-scheme-handler/{safe_scheme};\n"
    ))
}

pub(super) fn register(scheme: &str, exe: &str) -> io::Result<()> {
    let path = desktop_file_path()?;
    let name = app_id();
    let body = render_desktop_entry(&name, exe, scheme)?;
    let tmp = path.with_extension("desktop.tmp");
    {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(body.as_bytes())?;
        f.sync_all()?;
    }
    fs::rename(&tmp, &path)?;

    let id = format!("{}.desktop", app_id());
    let mime = format!("x-scheme-handler/{scheme}");
    // The .desktop file alone does not route links: without the xdg-mime
    // association (missing xdg-utils, failing command) nothing would open
    // the app, so report it instead of claiming success.
    let status = Command::new("xdg-mime")
        .args(["default", &id, &mime])
        .status()
        .map_err(|e| io::Error::new(e.kind(), format!("xdg-mime: {e}")))?;
    if !status.success() {
        return Err(io::Error::other(format!(
            "xdg-mime default {id} {mime} failed ({status})"
        )));
    }
    if let Some(dir) = applications_dir()
        .ok()
        .and_then(|p| p.to_str().map(String::from))
    {
        let _ = Command::new("update-desktop-database").arg(dir).status();
    }
    Ok(())
}

pub(super) fn unregister(_scheme: &str) -> io::Result<()> {
    let path = desktop_file_path()?;
    match fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

pub(super) fn is_registered(scheme: &str) -> io::Result<bool> {
    let mimeapps = mimeapps_list_path()?;
    let key = format!("x-scheme-handler/{scheme}=");
    let expected = format!("{}.desktop", app_id());

    let f = match fs::File::open(&mimeapps) {
        Ok(f) => f,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e),
    };
    let reader = BufReader::new(f);
    let mut in_default_section = false;
    for line in reader.lines() {
        let line = line?;
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_default_section = trimmed.eq_ignore_ascii_case("[Default Applications]");
            continue;
        }
        if !in_default_section {
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix(&key) {
            return Ok(rest.split(';').any(|v| v == expected));
        }
    }
    Ok(false)
}

fn mimeapps_list_path() -> io::Result<PathBuf> {
    let base = if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        PathBuf::from(xdg)
    } else {
        let home = dirs::home_dir()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "could not locate $HOME"))?;
        home.join(".config")
    };
    Ok(base.join("mimeapps.list"))
}
