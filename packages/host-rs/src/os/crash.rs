//! Panic -> file crash reporter. Opt-in per app id. Writes one
//! `crash-<unix-nanos>.log` per panic under `data_dir/<app_id>/crashes/`.

use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;
use std::sync::OnceLock;

static INSTALLED: OnceLock<String> = OnceLock::new();

pub fn dispatch(method: &str, args: &Value) -> Result<Value, String> {
    match method {
        "enable" => enable(args),
        "logDir" => {
            let id = INSTALLED
                .get()
                .cloned()
                .ok_or_else(|| "crashReporter.logDir: not enabled".to_string())?;
            Ok(Value::String(
                crash_dir(&id)?.to_string_lossy().into_owned(),
            ))
        },
        "listCrashes" => {
            let id = INSTALLED
                .get()
                .cloned()
                .ok_or_else(|| "crashReporter.listCrashes: not enabled".to_string())?;
            let dir = crash_dir(&id)?;
            if !dir.exists() {
                return Ok(Value::Array(Vec::new()));
            }
            let mut out = Vec::new();
            for entry in fs::read_dir(&dir).map_err(|e| e.to_string())? {
                let entry = entry.map_err(|e| e.to_string())?;
                let name = entry.file_name().to_string_lossy().into_owned();
                let ext_is_log = std::path::Path::new(&name)
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("log"));
                if name.starts_with("crash-") && ext_is_log {
                    out.push(Value::String(entry.path().to_string_lossy().into_owned()));
                }
            }
            Ok(Value::Array(out))
        },
        _ => Err(format!("crashReporter.{method}: unknown method")),
    }
}

fn enable(args: &Value) -> Result<Value, String> {
    let app_id = args
        .get("appId")
        .and_then(Value::as_str)
        .ok_or_else(|| "crashReporter.enable: missing 'appId'".to_string())?;

    if INSTALLED.set(app_id.to_string()).is_err() {
        return Ok(json!({ "enabled": true, "already": true }));
    }

    let dir = crash_dir(app_id)?;
    let _ = fs::create_dir_all(&dir);

    let app_id_owned = app_id.to_string();
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        write_crash(&app_id_owned, info);
        prev(info);
    }));

    Ok(json!({ "enabled": true, "already": false, "dir": dir.to_string_lossy() }))
}

fn crash_dir(app_id: &str) -> Result<PathBuf, String> {
    let base = dirs::data_dir()
        .or_else(dirs::config_dir)
        .ok_or_else(|| "crashReporter: no data/config dir".to_string())?;
    Ok(base.join(app_id).join("crashes"))
}

fn write_crash(app_id: &str, info: &std::panic::PanicHookInfo<'_>) {
    let Ok(dir) = crash_dir(app_id) else { return };
    let _ = fs::create_dir_all(&dir);

    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let file = dir.join(format!("crash-{ts}.log"));

    let payload = format!(
        "tynd crash report\n\
         timestamp: {ts}\n\
         platform:  {} / {}\n\
         version:   tynd-host {}\n\
         \n\
         panic: {info}\n",
        std::env::consts::OS,
        std::env::consts::ARCH,
        env!("CARGO_PKG_VERSION"),
    );
    let _ = fs::write(file, payload);
}
