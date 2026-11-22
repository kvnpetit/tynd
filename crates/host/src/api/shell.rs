//! Shell integration — open URLs, files, reveal-in-folder.

use std::path::Path;

use crate::ipc::error::IpcError;

use crate::{api::HostApi, security::policy, tynd};

/// Build the `shell.<method>: <msg>` IPC error string.
#[cold]
fn shell_err(method: &str, msg: impl std::fmt::Display) -> String {
    IpcError::new("shell", method, msg).into_string()
}

/// Schemes accepted by [`ShellApi::open_external`]. Anything outside the
/// list is rejected with an `IpcError`. Kept narrow on purpose — every
/// extra scheme is a potential pivot through the OS protocol-handler
/// registry (`file://`, `vscode://`, ...).
const ALLOWED_EXTERNAL_SCHEMES: &[&str] = &["http", "https", "mailto"];

/// Stateless dispatch marker.
#[derive(Debug, Default)]
pub(crate) struct ShellApi;

/// Row of the host API table (`api::HOST_APIS`).
pub(crate) const API: HostApi = HostApi::new("shell", |method, args| dispatch(method, &args));

#[tynd::handler(api = "shell")]
impl ShellApi {
    /// Open `url` in the user's default browser. Only schemes in
    /// [`ALLOWED_EXTERNAL_SCHEMES`] (`http`, `https`, `mailto`) are
    /// accepted — anything else is rejected so a frontend bug or XSS
    /// can't pivot through `shell.openExternal` to invoke a custom
    /// protocol handler. The URL is parsed (not prefix-matched) so
    /// inputs like `"https:..//attacker.example"` or schemes that
    /// merely *contain* `http` don't slip through.
    pub fn open_external(url: String) -> Result<(), String> {
        let parsed = url::Url::parse(&url)
            .map_err(|e| shell_err("openExternal", format!("invalid URL '{url}': {e}")))?;
        let scheme = parsed.scheme();
        if !ALLOWED_EXTERNAL_SCHEMES.contains(&scheme) {
            return Err(shell_err(
                "openExternal",
                format!(
                    "unsupported scheme '{scheme}' in '{url}' — only {} allowed",
                    ALLOWED_EXTERNAL_SCHEMES.join(", ")
                ),
            ));
        }
        open::that(&url).map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Open `path` with the OS's default handler. Known executable /
    /// script / launcher file types are refused on every OS so that a
    /// frontend bug or XSS can't turn `shell.openPath` into a
    /// remote-code-execution primitive (write a file, then open it) —
    /// callers that need to launch a binary should use `process.exec`
    /// instead (which goes through capability checks).
    pub fn open_path(path: String) -> Result<(), String> {
        let resolved = policy::authorize_fs_path(&path).map_err(|e| shell_err("openPath", e))?;
        refuse_windows_special_path(Path::new(&path), &resolved)
            .map_err(|e| shell_err("openPath", e))?;
        if is_executable_name(&resolved) {
            return Err(shell_err(
                "openPath",
                format!("refusing to open executable '{path}' — use process.exec for binaries"),
            ));
        }
        open::that(&resolved).map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Open the OS file manager with `path` selected/highlighted.
    /// Uses the native "reveal" gesture (Explorer `/select`, Finder
    /// `open -R`, `xdg-open` parent on Linux) so the user sees the
    /// item in-place.
    pub fn reveal_in_folder(path: String) -> Result<(), String> {
        // SECURITY: gate the target path through the deployer's FS allow-
        // list, mirroring `open_path`. Without this, a `defaultDeny` policy
        // would still let a frontend bug ask the OS file manager to surface
        // an arbitrary path (and thereby leak existence / metadata).
        let resolved =
            policy::authorize_fs_path(&path).map_err(|e| shell_err("revealInFolder", e))?;
        refuse_windows_special_path(Path::new(&path), &resolved)
            .map_err(|e| shell_err("revealInFolder", e))?;
        let resolved = resolved
            .to_str()
            .ok_or_else(|| shell_err("revealInFolder", "resolved path is not valid UTF-8"))?;
        reveal(resolved)
    }
}

/// Open the OS file manager with `path` selected / highlighted. Uses the
/// native "reveal" gesture (explorer /select, Finder open -R, xdg-open
/// parent) so the user sees the item in-place.
#[cfg(target_os = "windows")]
fn reveal(path: &str) -> Result<(), String> {
    let p = Path::new(path);
    if !p.exists() {
        return Err(shell_err(
            "revealInFolder",
            format!("({path}): path does not exist"),
        ));
    }
    std::process::Command::new("explorer")
        .arg(format!("/select,{}", p.display()))
        .spawn()
        .map_err(|e| shell_err("revealInFolder", e))?;
    Ok(())
}

#[cfg(target_os = "macos")]
fn reveal(path: &str) -> Result<(), String> {
    if !Path::new(path).exists() {
        return Err(shell_err(
            "revealInFolder",
            format!("({path}): path does not exist"),
        ));
    }
    std::process::Command::new("open")
        .args(["-R", path])
        .spawn()
        .map_err(|e| shell_err("revealInFolder", e))?;
    Ok(())
}

#[cfg(all(unix, not(target_os = "macos")))]
fn reveal(path: &str) -> Result<(), String> {
    let p = Path::new(path);
    if !p.exists() {
        return Err(shell_err(
            "revealInFolder",
            format!("({path}): path does not exist"),
        ));
    }
    // No single Linux convention — open the parent dir, works on every
    // desktop that has `xdg-open` installed.
    let parent = p.parent().unwrap_or(p);
    open::that(parent).map_err(|e| shell_err("revealInFolder", e))?;
    Ok(())
}

/// File types whose default handler runs code (or installs/launches
/// something). Compared lowercase against the final extension of the file name.
const EXEC_EXTS: &[&str] = &[
    // Windows executables, scripts, shortcuts and launchers.
    "exe",
    "bat",
    "cmd",
    "com",
    "scr",
    "dll",
    "pif",
    "cpl",
    "msc",
    "msi",
    "msp",
    "jar",
    "ps1",
    "psm1",
    "psc1",
    "vbs",
    "vbe",
    "js",
    "jse",
    "wsf",
    "wsh",
    "wsc",
    "sct",
    "hta",
    "lnk",
    "url",
    "scf",
    "reg",
    "inf",
    "xll",
    "gadget",
    "appref-ms",
    "settingcontent-ms",
    "application",
    "chm",
    "search-ms",
    "library-ms",
    "diagcab",
    "ocx",
    "sys",
    "ws",
    "wsb",
    "rdp",
    "theme",
    "themepack",
    "website",
    "xbap",
    "mht",
    "mhtml",
    "jnlp",
    "appinstaller",
    "msix",
    "msixbundle",
    "appx",
    "appxbundle",
    "searchconnector-ms",
    "accde",
    "mde",
    "adp",
    "accdb",
    "accdr",
    "vsto",
    "cer",
    "crt",
    "der",
    "p7b",
    // Interpreter-associated scripts (py launcher on Windows, desktop
    // associations elsewhere).
    "py",
    "pyw",
    "pyz",
    "pyzw",
    "pl",
    "pyc",
    "pyo",
    "pyd",
    "rb",
    "rbw",
    "php",
    "phar",
    "lua",
    "tcl",
    "ps1xml",
    "ps2",
    "ps2xml",
    "psc2",
    "pssc",
    "cdxml",
    "msh",
    "msh1",
    "msh2",
    "mshxml",
    "msh1xml",
    "msh2xml",
    "drv",
    "msu",
    "mst",
    "mof",
    "ahk",
    "au3",
    "vb",
    // Disk images: mounting bypasses mark-of-the-web.
    "iso",
    "img",
    "vhd",
    "vhdx",
    // macOS bundles, scripts and installers.
    "app",
    "command",
    "tool",
    "terminal",
    "workflow",
    "action",
    "scpt",
    "scptd",
    "pkg",
    "mpkg",
    "dmg",
    "fileloc",
    "webloc",
    "inetloc",
    "mobileconfig",
    "shortcut",
    "prefpane",
    "saver",
    "plugin",
    "bundle",
    "qlgenerator",
    "kext",
    // Linux launchers and scripts.
    "desktop",
    "sh",
    "run",
    "appimage",
    "bash",
    "zsh",
    "csh",
    "ksh",
];

/// True when the file's effective extension is an executable type. The
/// shell resolves the handler from the FINAL extension, so `app.js.map` and
/// `script.exe.txt` open with the `.map` / `.txt` handler and are allowed.
/// Windows ignores trailing dots/spaces and NTFS alternate data streams
/// (`x.hta::$DATA`), so the name is cut at the first `:` and trimmed before
/// the extension is taken.
fn is_executable_name(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    let name = name.split(':').next().unwrap_or("");
    let name = name.trim_end_matches([' ', '.']);
    let Some((_, ext)) = name.rsplit_once('.') else {
        return false;
    };
    EXEC_EXTS.contains(&ext.to_ascii_lowercase().as_str())
}

/// True when the final path component names an NTFS alternate data stream
/// (`x.txt:s.vbs`). The shell picks the handler from the stream name, so the
/// extension check on the base name proves nothing.
fn names_alternate_stream(path: &Path) -> bool {
    // Split on both separators by hand so the check is identical on every OS.
    path.to_str().is_some_and(|text| {
        text.rsplit(['/', '\\'])
            .next()
            .is_some_and(|name| name.contains(':'))
    })
}

/// True for Win32 device namespace paths (`\\.\...`) and for `\\?\` paths that
/// do not point at a drive or UNC share (`\\?\GLOBALROOT\...`, `\\?\pipe\...`).
fn is_device_path(path: &str) -> bool {
    let path = path.replace('/', "\\");
    if path.starts_with(r"\\.\") {
        return true;
    }
    let Some(rest) = path.strip_prefix(r"\\?\") else {
        return false;
    };
    let bytes = rest.as_bytes();
    let drive =
        bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && bytes[2] == b'\\';
    let unc = rest
        .get(..4)
        .is_some_and(|p| p.eq_ignore_ascii_case(r"UNC\"));
    !(drive || unc)
}

/// Windows-only gate for `openPath` / `revealInFolder`: refuse alternate data
/// streams and device paths, in the caller's spelling and in the canonical
/// (long-name) path the policy resolved. Elsewhere `:` is an ordinary
/// file-name character.
fn refuse_windows_special_path(input: &Path, resolved: &Path) -> Result<(), String> {
    if !cfg!(windows) {
        return Ok(());
    }
    check_special_path(input, resolved)
}

fn check_special_path(input: &Path, resolved: &Path) -> Result<(), String> {
    for candidate in [input, resolved] {
        if names_alternate_stream(candidate) {
            return Err(format!(
                "refusing to open '{}': NTFS alternate data streams are not allowed",
                input.display()
            ));
        }
        if candidate.to_str().is_some_and(is_device_path) {
            return Err(format!(
                "refusing to open '{}': device paths are not allowed",
                input.display()
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{check_special_path, is_executable_name};

    #[test]
    fn refuses_alternate_streams_and_device_paths() {
        for name in [
            r"C:\Users\u\notes.txt:run.vbs",
            r"C:\Users\u\notes.txt:run.vbs:$DATA",
            r"C:\Users\u\a.pdf:b.bat",
            r"C:\Users\u\a.txt::$DATA",
            r"\\?\C:\Users\u\x.txt:evil.hta",
            r"\\.\C:\Users\u\x.txt",
            r"\\.\pipe\x",
            r"\\?\GLOBALROOT\Device\x",
            "//./C:/x.txt",
        ] {
            let p = Path::new(name);
            assert!(check_special_path(p, p).is_err(), "{name}");
        }
        // The resolved spelling alone is enough to refuse.
        assert!(check_special_path(Path::new("a.txt"), Path::new(r"C:\d\a.txt:s.py")).is_err());
    }

    #[test]
    fn allows_plain_drive_and_unc_paths() {
        for name in [
            r"C:\Users\u\notes.txt",
            r"\\?\C:\Users\u\notes.txt",
            r"\\?\UNC\server\share\a.pdf",
            r"\\server\share\a.pdf",
            "report.pdf",
        ] {
            let p = Path::new(name);
            assert!(check_special_path(p, p).is_ok(), "{name}");
        }
    }

    #[test]
    fn refuses_launcher_and_script_types_on_every_os() {
        for name in [
            "x.exe",
            "x.HTA",
            "x.searchConnector-ms",
            "x.ACCDE",
            "x.mde",
            "x.adp",
            "x.accdb",
            "x.accdr",
            "x.vsto",
            "x.cer",
            "x.CRT",
            "x.der",
            "x.p7b",
            "x.mobileconfig",
            "x.shortcut",
            "x.lnk",
            "x.url",
            "x.js",
            "x.reg",
            "x.msc",
            "x.appref-ms",
            "x.settingcontent-ms",
            "x.chm",
            "x.command",
            "x.terminal",
            "x.workflow",
            "x.pkg",
            "Tool.app",
            "x.desktop",
            "x.sh",
            "Editor.AppImage",
            "x.run",
            "x.pyw",
            "x.py",
            "x.pyc",
            "x.PYO",
            "x.pyz",
            "x.pyzw",
            "x.rb",
            "x.pl",
            "x.php",
            "x.lua",
            "x.tcl",
            "x.ps1xml",
            "x.msh",
            "x.msh1xml",
            "x.cpl",
            "x.ocx",
            "x.sys",
            "x.drv",
            "x.gadget",
            "x.xbap",
            "x.xll",
            "x.wsc",
            "x.wsh",
            "x.jnlp",
            "x.appinstaller",
            "x.fileloc",
            "x.webloc",
            "x.application",
            "x.library-ms",
            "x.scf",
            "x.msix",
            "x.appx",
            "x.wsb",
            "x.rdp",
            "x.dmg",
            "x.prefPane",
            "x.iso",
        ] {
            assert!(is_executable_name(Path::new(name)), "{name}");
        }
    }

    #[test]
    fn refuses_embedded_and_obfuscated_extensions() {
        for name in [
            "x.hta ",
            "x.hta.",
            "x.hta. .",
            "x.hta::$DATA",
            "evil.exe.",
            "evil.exe::$DATA",
            "evil.exe:stream:$DATA",
            "a.b.lnk",
        ] {
            assert!(is_executable_name(Path::new(name)), "{name}");
        }
    }

    #[test]
    fn allows_documents() {
        for name in [
            "report.pdf",
            "photo.png",
            "notes.txt",
            "archive.tar.gz",
            "README",
            "data.json",
            "x.js.map",
            "app.js.map",
            "script.exe.txt",
            ".bashrc",
        ] {
            assert!(!is_executable_name(Path::new(name)), "{name}");
        }
    }
}
