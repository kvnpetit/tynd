use notify_rust::Notification;
use serde_json::Value;

pub fn dispatch(method: &str, args: &Value) -> Result<Value, String> {
    match method {
        "send" => send(args),
        _ => Err(format!("notification.{method}: unknown method")),
    }
}

fn send(args: &Value) -> Result<Value, String> {
    let title = args.get("title").and_then(|t| t.as_str()).unwrap_or("");
    let body = args.get("body").and_then(|b| b.as_str()).unwrap_or("");

    let app_name = std::env::current_exe()
        .ok()
        .and_then(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "vorn".to_string());

    Notification::new()
        .appname(&app_name)
        .summary(title)
        .body(body)
        .show()
        .map_err(|e| e.to_string())?;

    Ok(Value::Null)
}
