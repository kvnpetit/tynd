use serde_json::{json, Value};

use super::events;

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

    send_platform(title, body, icon, sound, actions)
}

fn emit_action(id: &str) {
    events::emit("notification:action", &json!({ "action": id }));
}

// ---- Linux (libnotify via notify-rust) ----------------------------------

#[cfg(all(unix, not(target_os = "macos")))]
#[allow(clippy::needless_pass_by_value)] // signature stays uniform across OS branches
fn send_platform(
    title: &str,
    body: &str,
    icon: Option<&str>,
    sound: Option<&str>,
    actions: Vec<(String, String)>,
) -> Result<Value, String> {
    use notify_rust::Notification;

    let app_name = exe_stem();
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

    let handle = n.show().map_err(|e| e.to_string())?;
    if !actions.is_empty() {
        std::thread::spawn(move || {
            handle.wait_for_action(|action| emit_action(action));
        });
    }
    Ok(Value::Null)
}

// ---- Windows (WinRT toasts via tauri-winrt-notification) -----------------

#[cfg(target_os = "windows")]
#[allow(clippy::needless_pass_by_value)] // signature stays uniform across OS branches
fn send_platform(
    title: &str,
    body: &str,
    icon: Option<&str>,
    sound: Option<&str>,
    actions: Vec<(String, String)>,
) -> Result<Value, String> {
    use std::str::FromStr as _;
    use tauri_winrt_notification::{IconCrop, Sound, Toast};

    let app_id = exe_stem();
    let mut toast = Toast::new(&app_id).title(title).text1(body);

    if let Some(p) = icon {
        toast = toast.icon(std::path::Path::new(p), IconCrop::Square, "");
    }
    if let Some(s) = sound {
        if let Ok(snd) = Sound::from_str(s) {
            toast = toast.sound(Some(snd));
        }
    }
    for (id, label) in &actions {
        // `action` arg on the button is what `on_activated` receives when
        // the user clicks — use our public id as the payload.
        toast = toast.add_button(label, id);
    }
    if !actions.is_empty() {
        toast = toast.on_activated(move |arg| {
            if let Some(action) = arg {
                emit_action(&action);
            }
            Ok(())
        });
    }

    toast.show().map_err(|e| format!("notification: {e:?}"))?;
    Ok(Value::Null)
}

// ---- macOS (NSUserNotificationCenter via mac-notification-sys) ----------

#[cfg(target_os = "macos")]
fn send_platform(
    title: &str,
    body: &str,
    icon: Option<&str>,
    sound: Option<&str>,
    actions: Vec<(String, String)>,
) -> Result<Value, String> {
    use mac_notification_sys::{MainButton, Notification, NotificationResponse};

    // Owned strings so we can move them into the action-waiting thread.
    let title_owned = title.to_string();
    let body_owned = body.to_string();
    let icon_owned = icon.map(str::to_string);
    let sound_owned = sound.map(str::to_string);

    let build = move |actions: &[(String, String)]| -> Result<NotificationResponse, String> {
        let labels: Vec<&str> = actions.iter().map(|(_, l)| l.as_str()).collect();
        let mut n = Notification::new();
        n.title(&title_owned).message(&body_owned);
        if let Some(ref s) = sound_owned {
            n.sound(s);
        }
        if let Some(ref p) = icon_owned {
            n.content_image(p);
        }
        match labels.len() {
            0 => {},
            1 => {
                n.main_button(MainButton::SingleAction(labels[0]));
            },
            _ => {
                n.main_button(MainButton::DropdownActions("Actions", &labels));
            },
        }
        n.send().map_err(|e| e.to_string())
    };

    if actions.is_empty() {
        let _ = build(&[])?;
    } else {
        // `send()` blocks until the user clicks or the banner is dismissed.
        // Off-thread so the call pool keeps serving unrelated requests.
        std::thread::spawn(move || {
            if let Ok(response) = build(&actions) {
                if let NotificationResponse::ActionButton(label) = response {
                    if let Some((id, _)) = actions.iter().find(|(_, l)| *l == label) {
                        emit_action(id);
                    }
                }
            }
        });
    }
    Ok(Value::Null)
}

fn exe_stem() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "tynd".to_string())
}
