use notify_rust::Notification;
use serde_json::Value;

#[cfg(all(unix, not(target_os = "macos")))]
use super::events;
#[cfg(all(unix, not(target_os = "macos")))]
use serde_json::json;

pub fn dispatch(method: &str, args: &Value) -> Result<Value, String> {
    match method {
        "send" => send(args),
        _ => Err(format!("notification.{method}: unknown method")),
    }
}

fn send(args: &Value) -> Result<Value, String> {
    let title = args.get("title").and_then(Value::as_str).unwrap_or("");
    let body = args.get("body").and_then(Value::as_str).unwrap_or("");
    let icon = args.get("icon").and_then(Value::as_str);
    let sound = args.get("sound").and_then(Value::as_str);
    let actions: Vec<(String, String)> = args
        .get("actions")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|v| {
                    let id = v.get("id").and_then(Value::as_str)?;
                    let label = v.get("label").and_then(Value::as_str)?;
                    Some((id.to_string(), label.to_string()))
                })
                .collect()
        })
        .unwrap_or_default();

    let app_name = std::env::current_exe()
        .ok()
        .and_then(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "tynd".to_string());

    let mut n = Notification::new();
    n.appname(&app_name).summary(title).body(body);
    if let Some(p) = icon {
        n.icon(p);
    }
    if let Some(s) = sound {
        n.sound_name(s);
    }
    for (id, label) in &actions {
        n.action(id, label);
    }

    // Wait-for-action runs an async loop on xdg — move to a thread so the
    // call pool is never parked. Win/macOS don't surface clicks through this
    // API, so the `notification:action` event only fires on Linux today.
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let handle = n.show().map_err(|e| e.to_string())?;
        if !actions.is_empty() {
            std::thread::spawn(move || {
                handle.wait_for_action(|action| {
                    events::emit("notification:action", &json!({ "action": action }));
                });
            });
        }
    }
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    {
        let _ = &actions;
        n.show().map_err(|e| e.to_string())?;
    }

    Ok(Value::Null)
}
