//! Window-to-window messaging (`multiwindow.events`): a renderer emits an
//! event that the host delivers to one webview (`emitTo`) or to every
//! app-local webview (`emit`) as a `multiwindow:message` OS event.

use serde_json::{json, Value};

use super::{dispatch_support::broadcast_script_scoped, runtime::WindowRegistry};
use crate::{
    ipc::events::EventScope,
    webview::{handlers::is_app_local_webview, shim},
};

/// OS event carrying `{ event, payload, from }` to the receiving renderers.
const WINDOW_MESSAGE_EVENT: &str = "multiwindow:message";
const MAX_EVENT_NAME_LEN: usize = 128;

/// Arguments of a validated `emit` / `emitTo` call.
#[derive(Debug, PartialEq)]
pub(super) struct WindowMessage<'a> {
    pub(super) to: Option<&'a str>,
    pub(super) event: &'a str,
    pub(super) payload: &'a Value,
}

pub(super) fn parse_window_message<'a>(
    method: &str,
    args: &'a Value,
    caller_origin: Option<&str>,
) -> Result<WindowMessage<'a>, String> {
    if caller_origin.is_some() {
        return Err(format!(
            "window.{method}: remote content cannot message other windows"
        ));
    }
    let event = args
        .get("event")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("window.{method}: missing 'event'"))?;
    let valid_name = !event.is_empty()
        && event.len() <= MAX_EVENT_NAME_LEN
        && event
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_-:./".contains(&byte));
    if !valid_name {
        return Err(format!(
            "window.{method}: 'event' must be 1-{MAX_EVENT_NAME_LEN} ASCII letters, digits, \
             '_', '-', ':', '.' or '/'"
        ));
    }
    let to = if method == "emitTo" {
        Some(
            args.get("to")
                .and_then(Value::as_str)
                .ok_or_else(|| "window.emitTo: missing 'to' (target window label)".to_string())?,
        )
    } else {
        None
    };
    Ok(WindowMessage {
        to,
        event,
        payload: args.get("payload").unwrap_or(&Value::Null),
    })
}

/// Deliver `message` from the webview `from`. Remote-content webviews never
/// receive messages: app code is the only sender.
pub(super) fn deliver_window_message(
    windows: &WindowRegistry,
    from: &str,
    message: &WindowMessage<'_>,
) -> Result<Value, String> {
    let data = json!({ "event": message.event, "payload": message.payload, "from": from });
    let script = shim::eval_os_event(WINDOW_MESSAGE_EVENT, &data);
    let Some(to) = message.to else {
        broadcast_script_scoped(
            &script,
            &EventScope::Local,
            &windows.webview,
            &windows.secondaries,
            &windows.child_webviews,
        );
        return Ok(Value::Null);
    };
    let webview = windows
        .webview_for(to)
        .ok_or_else(|| format!("window.emitTo: window '{to}' not found"))?;
    if !is_app_local_webview(to) {
        return Err(format!(
            "window.emitTo: '{to}' shows remote content and cannot receive window messages"
        ));
    }
    webview
        .evaluate_script(&script)
        .map_err(|error| format!("window.emitTo: {error}"))?;
    Ok(Value::Null)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn emit_to_needs_a_target_and_a_safe_event_name() {
        let args = json!({ "to": "settings", "event": "doc:saved", "payload": { "id": 1 } });
        let message = parse_window_message("emitTo", &args, None).unwrap();
        assert_eq!(message.to, Some("settings"));
        assert_eq!(message.event, "doc:saved");
        assert_eq!(message.payload, &json!({ "id": 1 }));

        let error = parse_window_message("emitTo", &json!({ "event": "x" }), None).unwrap_err();
        assert!(error.contains("'to'"), "{error}");
        for bad in ["", "has space", "quote\"", &"x".repeat(129)] {
            let error = parse_window_message("emit", &json!({ "event": bad }), None).unwrap_err();
            assert!(error.contains("'event'"), "{bad}: {error}");
        }
    }

    #[test]
    fn emit_broadcasts_without_target_and_defaults_payload_to_null() {
        let args = json!({ "event": "refresh" });
        let message = parse_window_message("emit", &args, None).unwrap();
        assert_eq!(message.to, None);
        assert_eq!(message.payload, &Value::Null);
    }

    #[test]
    fn remote_content_cannot_send_messages() {
        let error = parse_window_message(
            "emit",
            &json!({ "event": "x" }),
            Some("https://example.com"),
        )
        .unwrap_err();
        assert!(error.contains("remote content"), "{error}");
    }
}
