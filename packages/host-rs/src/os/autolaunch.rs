//! Launch-at-boot toggle — wraps the `auto-launch` crate for a uniform TS API.
//!
//! Per-platform storage:
//! - Windows: `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`
//! - macOS:   Launch Agent plist under `~/Library/LaunchAgents/<bundle>.plist`
//! - Linux:   `.desktop` entry under `~/.config/autostart/<name>.desktop`

use auto_launch::AutoLaunchBuilder;
use serde_json::{json, Value};

pub fn dispatch(method: &str, args: &Value) -> Result<Value, String> {
    match method {
        "enable" => enable(args),
        "disable" => disable(args),
        "isEnabled" => is_enabled(args),
        _ => Err(format!("autolaunch.{method}: unknown method")),
    }
}

fn resolved_name(args: &Value) -> Result<String, String> {
    if let Some(n) = args.get("name").and_then(Value::as_str) {
        return Ok(n.to_string());
    }
    std::env::current_exe()
        .ok()
        .and_then(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
        .ok_or_else(|| "autolaunch: cannot derive app name from current_exe".to_string())
}

fn exe_path() -> Result<String, String> {
    std::env::current_exe()
        .map(|p| p.to_string_lossy().into_owned())
        .map_err(|e| format!("autolaunch: current_exe: {e}"))
}

fn build(args: &Value) -> Result<auto_launch::AutoLaunch, String> {
    let name = resolved_name(args)?;
    let path = exe_path()?;
    let extra: Vec<String> = args
        .get("args")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(Value::as_str)
                .map(String::from)
                .collect()
        })
        .unwrap_or_default();
    let extra_refs: Vec<&str> = extra.iter().map(String::as_str).collect();

    AutoLaunchBuilder::new()
        .set_app_name(&name)
        .set_app_path(&path)
        .set_args(&extra_refs)
        .build()
        .map_err(|e| format!("autolaunch.build: {e}"))
}

fn enable(args: &Value) -> Result<Value, String> {
    let al = build(args)?;
    al.enable().map_err(|e| format!("autolaunch.enable: {e}"))?;
    Ok(json!({ "enabled": true }))
}

fn disable(args: &Value) -> Result<Value, String> {
    let al = build(args)?;
    al.disable()
        .map_err(|e| format!("autolaunch.disable: {e}"))?;
    Ok(json!({ "enabled": false }))
}

fn is_enabled(args: &Value) -> Result<Value, String> {
    let al = build(args)?;
    let enabled = al
        .is_enabled()
        .map_err(|e| format!("autolaunch.isEnabled: {e}"))?;
    Ok(Value::Bool(enabled))
}
