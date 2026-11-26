//! Launch-at-boot toggle — wraps the `auto-launch` crate for a uniform
//! TS API across platforms.
//!
//! Per-platform storage:
//! - Windows: `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`
//! - macOS:   Launch Agent plist under `~/Library/LaunchAgents/<bundle>.plist`
//! - Linux:   `.desktop` entry under `~/.config/autostart/<name>.desktop`

use auto_launch::AutoLaunchBuilder;

use crate::api::HostApi;
use crate::ipc::error::IpcError;
use crate::tynd;

/// Dispatch marker — host is stateless; the OS is source of truth.
#[derive(Debug, Default)]
pub(crate) struct AutolaunchApi;

/// Row of the host API table (`api::HOST_APIS`).
pub(crate) const API: HostApi = HostApi::new("autolaunch", |method, args| dispatch(method, &args));

#[tynd::handler(api = "autolaunch")]
impl AutolaunchApi {
    /// Register the running binary to launch at user login. `name`
    /// defaults to the current executable's filename. `args` is the
    /// extra argv passed to the launched binary.
    pub fn enable(name: Option<String>, args: Option<Vec<String>>) -> Result<bool, String> {
        let al = build(name, args)?;
        al.enable()
            .map_err(|e| IpcError::new("autolaunch", "enable", e).into_string())?;
        Ok(true)
    }

    /// Remove the launch-at-login registration. `name` defaults to the
    /// current executable's filename.
    pub fn disable(name: Option<String>, args: Option<Vec<String>>) -> Result<bool, String> {
        let al = build(name, args)?;
        al.disable()
            .map_err(|e| IpcError::new("autolaunch", "disable", e).into_string())?;
        Ok(false)
    }

    /// True when the running binary is registered to launch at login.
    pub fn is_enabled(name: Option<String>, args: Option<Vec<String>>) -> Result<bool, String> {
        let al = build(name, args)?;
        al.is_enabled()
            .map_err(|e| IpcError::new("autolaunch", "isEnabled", e).into_string())
    }
}

fn resolved_name(name: Option<String>) -> Result<String, String> {
    if let Some(n) = name {
        return Ok(n);
    }
    std::env::current_exe()
        .ok()
        .and_then(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
        .ok_or_else(|| "autolaunch: cannot derive app name from current_exe".to_string())
}

fn exe_path() -> Result<String, String> {
    std::env::current_exe()
        .map(|p| p.to_string_lossy().into_owned())
        .map_err(|e| {
            IpcError::new("autolaunch", "build", format!("current_exe: {e}")).into_string()
        })
}

fn build(
    name: Option<String>,
    args: Option<Vec<String>>,
) -> Result<auto_launch::AutoLaunch, String> {
    let name = resolved_name(name)?;
    let path = exe_path()?;
    let extra: Vec<String> = args.unwrap_or_default();
    let extra_refs: Vec<&str> = extra.iter().map(String::as_str).collect();

    AutoLaunchBuilder::new()
        .set_app_name(&name)
        .set_app_path(&path)
        .set_args(&extra_refs)
        .build()
        .map_err(|e| IpcError::new("autolaunch", "build", e).into_string())
}
