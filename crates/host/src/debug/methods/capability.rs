//! `capability.*` debug methods — list, check, suggest_grant.

use serde_json::{json, Value};
use tynd_debug_protocol::protocol::RpcError;

use super::CallOutcome;

pub(crate) fn list(_params: Value) -> CallOutcome {
    let caps = crate::security::acl::authority().list();
    let serialized: Vec<Value> = caps
        .iter()
        .map(|c| serde_json::to_value(c).unwrap_or(Value::Null))
        .collect();
    CallOutcome::Result(json!({ "capabilities": serialized }))
}

pub(crate) fn check(params: Value) -> CallOutcome {
    let Some(cmd) = params
        .get("command")
        .or_else(|| params.get("api"))
        .and_then(Value::as_str)
    else {
        return CallOutcome::Error(RpcError::invalid_params("missing 'api'"));
    };
    let window = params.get("window").and_then(Value::as_str);
    let webview = params.get("webview").and_then(Value::as_str);
    let origin = params.get("origin").and_then(Value::as_str);
    let allowed = crate::security::acl::authority().is_allowed(cmd, window, webview, origin);
    CallOutcome::Result(json!({ "command": cmd, "allowed": allowed }))
}

pub(crate) fn suggest_grant(params: Value) -> CallOutcome {
    let command = params
        .get("command")
        .or_else(|| params.get("api"))
        .and_then(Value::as_str)
        .unwrap_or("app:command");
    let window = params
        .get("window")
        .and_then(Value::as_str)
        .unwrap_or("main");
    let identifier = suggestion_identifier(command);
    CallOutcome::Result(json!({
        "suggestion": {
            "identifier": identifier,
            "description": format!("Debug grant for {command}."),
            "windows": [window],
            "permissions": [{
                "identifier": format!("allow-{identifier}"),
                "description": format!("Allows the '{command}' command."),
                "commands": { "allow": [command], "deny": [] },
            }],
        },
        "note": "add this capability to app.start({ capabilities: [...] }) or load it from a trusted *.tynd-capability.json file",
    }))
}

fn suggestion_identifier(command: &str) -> String {
    const PREFIX: &str = "debug-grant-";
    let mut suffix = String::with_capacity(64 - PREFIX.len());
    for character in command.chars() {
        let character = character.to_ascii_lowercase();
        if character.is_ascii_alphanumeric() {
            suffix.push(character);
        } else if !suffix.ends_with('-') && !suffix.is_empty() {
            suffix.push('-');
        }
        if suffix.len() >= 64 - PREFIX.len() {
            break;
        }
    }
    while suffix.ends_with('-') {
        suffix.pop();
    }
    if suffix.is_empty() {
        suffix.push_str("command");
    }
    format!("{PREFIX}{suffix}")
}
