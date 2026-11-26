use serde_json::{json, Value};

use crate::api::HostApi;
use crate::ipc::error::IpcError;

use super::{
    authorize_fs_path, check_fs, check_http, check_process, check_sensitive_capability, configure,
};

/// Row of the host API table (`api::HOST_APIS`).
pub(crate) const API: HostApi = HostApi::new("security", |method, args| dispatch(method, &args));

pub(crate) fn dispatch(method: &str, args: &Value) -> Result<Value, String> {
    // Runtime scope grants. Backend only: none of these is in the renderer /
    // plugin allow-list of `api::deny_privileged_mutation`.
    if let Some(result) = crate::security::grants::dispatch(method, args) {
        return result;
    }
    match method {
        "configure" => configure(args),
        "isFsAllowed" => is_fs_allowed(args),
        "checkFs" => require_fs_allowed(args),
        "isHttpAllowed" => is_http_allowed(args),
        "checkHttp" => require_http_allowed(args),
        "checkProcess" => require_process_allowed(args),
        "checkAudioInput" => check_sensitive_capability("audioInput"),
        "checkAudioOutput" => check_sensitive_capability("audioOutput"),
        "checkScreenCapture" => check_sensitive_capability("screenCapture"),
        "checkWindow" => check_sensitive_capability("window"),
        _ => Err(String::from(IpcError::unknown_method("security", method))),
    }
}

fn required_argument<'a>(args: &'a Value, method: &str, argument: &str) -> Result<&'a str, String> {
    args.get(argument)
        .and_then(Value::as_str)
        .ok_or_else(|| String::from(IpcError::missing_arg("security", method, argument)))
}

fn is_fs_allowed(args: &Value) -> Result<Value, String> {
    let path = required_argument(args, "isFsAllowed", "path")?;
    Ok(Value::Bool(check_fs(path).is_ok()))
}

fn require_fs_allowed(args: &Value) -> Result<Value, String> {
    let path = required_argument(args, "checkFs", "path")?;
    let resolved = authorize_fs_path(path)?;
    let resolved = resolved.to_str().ok_or_else(|| {
        format!(
            "security.checkFs: resolved path '{}' is not valid UTF-8",
            resolved.display()
        )
    })?;
    Ok(Value::String(resolved.to_string()))
}

fn is_http_allowed(args: &Value) -> Result<Value, String> {
    let url = required_argument(args, "isHttpAllowed", "url")?;
    Ok(Value::Bool(check_http(url).is_ok()))
}

fn require_http_allowed(args: &Value) -> Result<Value, String> {
    let url = required_argument(args, "checkHttp", "url")?;
    check_http(url)?;
    Ok(Value::Null)
}

fn require_process_allowed(args: &Value) -> Result<Value, String> {
    let binary = args
        .get("binary")
        .or_else(|| args.get("cmd"))
        .and_then(Value::as_str)
        .ok_or_else(|| String::from(IpcError::missing_arg("security", "checkProcess", "binary")))?;
    check_process(binary)?;
    let cwd = args
        .get("cwd")
        .and_then(Value::as_str)
        .map(authorize_fs_path)
        .transpose()?
        .map(|path| {
            path.into_os_string()
                .into_string()
                .map_err(|_| "security.checkProcess: resolved cwd is not valid UTF-8".to_string())
        })
        .transpose()?;
    Ok(json!({ "cwd": cwd }))
}
