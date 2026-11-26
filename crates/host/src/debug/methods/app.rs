//! `app.*` debug methods — info, quit, restart.

use serde_json::{json, Value};
use tynd_debug_protocol::protocol::RpcError;

use super::CallOutcome;

fn started_at_ms() -> u64 {
    static STARTED_AT: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    *STARTED_AT.get_or_init(|| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |duration| duration.as_millis() as u64)
    })
}

pub(crate) fn info(_params: Value) -> CallOutcome {
    let pid = std::process::id();
    let exe = std::env::current_exe()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    let host_version = env!("CARGO_PKG_VERSION");
    let started_at = started_at_ms();
    let app_name = crate::api::app::AppApi::get_name();
    let app_version = crate::api::app::AppApi::get_version();
    let windows = match super::window::list(Value::Null) {
        CallOutcome::Result(value) => value,
        _ => Value::Array(Vec::new()),
    };
    CallOutcome::Result(json!({
        "app_id": app_name,
        "pid": pid,
        "version": app_version,
        "tynd_version": host_version,
        "started_at": started_at,
        "windows": windows,
        "exe": exe,
        "hostVersion": host_version,
        "appName": app_name,
        "appVersion": app_version,
        "platform": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
    }))
}

pub(crate) fn quit(params: Value) -> CallOutcome {
    let code = params.get("code").and_then(Value::as_i64).unwrap_or(0) as i32;
    // Defer the exit so the response frame ships first.
    if let Err(error) = std::thread::Builder::new()
        .name("tynd-debug-quit".into())
        .spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(50));
            if code == 0 && crate::cleanup::trigger_shutdown() {
                return;
            }
            crate::cleanup::run();
            std::process::exit(code);
        })
    {
        return CallOutcome::Error(RpcError::internal_error(format!("quit scheduler: {error}")));
    }
    CallOutcome::Result(json!({ "queued": true, "code": code }))
}

pub(crate) fn restart(_params: Value) -> CallOutcome {
    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => return CallOutcome::Error(RpcError::internal_error(format!("current_exe: {e}"))),
    };
    let mut child = match std::process::Command::new(&exe).spawn() {
        Ok(child) => child,
        Err(error) => {
            return CallOutcome::Error(RpcError::internal_error(format!("spawn: {error}")));
        },
    };
    if let Err(error) = std::thread::Builder::new()
        .name("tynd-debug-restart".into())
        .spawn(|| {
            std::thread::sleep(std::time::Duration::from_millis(150));
            if !crate::cleanup::trigger_shutdown() {
                crate::cleanup::run();
                std::process::exit(0);
            }
        })
    {
        let _ = child.kill();
        let _ = child.wait();
        return CallOutcome::Error(RpcError::internal_error(format!(
            "restart scheduler: {error}"
        )));
    }
    CallOutcome::Result(json!({ "queued": true }))
}
