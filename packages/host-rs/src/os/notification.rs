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
    let body  = args.get("body").and_then(|b| b.as_str()).unwrap_or("");

    Notification::new()
        .appname("")
        .summary(title)
        .body(body)
        .show()
        .map_err(|e| e.to_string())?;

    Ok(Value::Null)
}
