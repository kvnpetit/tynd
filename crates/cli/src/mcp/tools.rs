use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Map, Value};

use super::runtime::Runtime;

const MAX_TOOL_TEXT_BYTES: usize = 4 * 1024 * 1024;

const TOOL_META: [(&str, &str, &str); 34] = [
    (
        "app.info",
        "App info",
        "Read host metadata for the attached tynd dev app.",
    ),
    (
        "app.quit",
        "Quit app",
        "Ask the attached tynd dev app to exit cleanly.",
    ),
    (
        "app.restart",
        "Restart app",
        "Restart the attached tynd dev app.",
    ),
    (
        "app.logs.tail",
        "Tail app logs",
        "Return the last N log frames emitted by the attached app.",
    ),
    (
        "app.launch",
        "Launch a tynd dev app",
        "Spawn `tynd dev` in an absolute project path. Gated by `--allow-launch`.",
    ),
    (
        "window.list",
        "List windows",
        "Enumerate every window the attached tynd app has open.",
    ),
    (
        "window.info",
        "Window info",
        "Read metadata for a specific window by label.",
    ),
    (
        "window.manage",
        "Manage a window",
        "Move, resize, focus, minimize, maximize, or close a window.",
    ),
    (
        "screenshot.window",
        "Capture a window screenshot",
        "Return a full-resolution PNG screenshot.",
    ),
    (
        "screenshot.thumbnail",
        "Capture a window thumbnail",
        "Return a small PNG thumbnail.",
    ),
    (
        "webview.snapshot",
        "Snapshot the webview",
        "Capture a compact map, raw HTML, or state snapshot.",
    ),
    (
        "webview.find_element",
        "Find an element",
        "Resolve a CSS selector to a ref and bounding box.",
    ),
    (
        "webview.console_logs",
        "Read webview console logs",
        "Return captured console output.",
    ),
    (
        "webview.network_requests",
        "List webview network requests",
        "Return captured webview network requests.",
    ),
    (
        "webview.click",
        "Click in the webview",
        "Click an element ref or absolute coordinates.",
    ),
    (
        "webview.type",
        "Type text",
        "Type into the focused element or a target ref.",
    ),
    (
        "webview.fill",
        "Fill form fields",
        "Fill several fields in one call.",
    ),
    (
        "webview.press_key",
        "Press a keyboard key",
        "Press a key combination such as Enter or Mod+s.",
    ),
    (
        "webview.scroll",
        "Scroll the webview",
        "Scroll by direction, amount, element, top, or bottom.",
    ),
    (
        "webview.hover",
        "Hover over an element",
        "Move the cursor over an element ref.",
    ),
    (
        "webview.drag",
        "Drag in the webview",
        "Drag from one ref or coordinate to another.",
    ),
    (
        "webview.evaluate",
        "Evaluate JavaScript",
        "Evaluate JavaScript. Gated by `--allow-eval`.",
    ),
    (
        "ipc.tap_start",
        "Start IPC tap",
        "Start streaming IPC frames into the host buffer.",
    ),
    ("ipc.tap_stop", "Stop IPC tap", "Stop the live IPC stream."),
    (
        "ipc.recent",
        "Recent IPC frames",
        "Return recent frames from the host IPC ring buffer.",
    ),
    (
        "ipc.failed",
        "Failed IPC frames",
        "Return IPC frames whose RPC ended in an error.",
    ),
    (
        "ipc.replay",
        "Replay an IPC frame",
        "Re-issue a previously captured IPC frame by id.",
    ),
    (
        "plugin.list",
        "List plugins",
        "List live plugins, or the bundled catalog in static mode.",
    ),
    (
        "plugin.docs",
        "Plugin docs",
        "Return bundled and optional live plugin documentation.",
    ),
    (
        "plugin.command_catalog",
        "Plugin command catalog",
        "Return a bundled generated plugin schema.",
    ),
    (
        "capability.list",
        "List capabilities",
        "Return the active host capability set.",
    ),
    (
        "capability.check",
        "Check capability",
        "Ask whether an API is currently authorized.",
    ),
    (
        "capability.suggest_grant",
        "Suggest capability grant",
        "Return a capability config snippet; works offline.",
    ),
    (
        "runtime.snapshot",
        "Runtime snapshot",
        "Capture workers, in-flight RPCs, and memory state.",
    ),
];

pub(super) fn list_tools() -> Value {
    Value::Array(
        TOOL_META
            .iter()
            .map(|(name, title, description)| {
                json!({
                    "name": name,
                    "title": title,
                    "description": description,
                    "inputSchema": input_schema(name),
                })
            })
            .collect(),
    )
}

pub(super) fn call_tool(runtime: &mut Runtime, name: &str, arguments: Value) -> Value {
    match call_tool_inner(runtime, name, arguments) {
        Ok(result) => result,
        Err(error) => error_result(&error.to_string()),
    }
}

fn call_tool_inner(runtime: &mut Runtime, name: &str, arguments: Value) -> Result<Value> {
    let args = object_or_empty(arguments)?;
    match name {
        "app.info" | "window.list" | "ipc.failed" | "capability.list" | "runtime.snapshot" => {
            live_text(runtime, name, Value::Null)
        },
        "app.quit" | "app.restart" | "ipc.tap_start" | "ipc.tap_stop" => {
            runtime.request_live(name, Value::Null)?;
            json_text(json!({ "ok": true }))
        },
        "app.logs.tail" => {
            let n = optional_u64(&args, "n", 100, 1, 1000)?;
            live_text(runtime, name, json!({ "n": n }))
        },
        "app.launch" => {
            let path = required_string(&args, "path", 32_768)?;
            json_text(runtime.launch(path)?)
        },
        "window.info" => {
            let label = required_string(&args, "label", 256)?;
            live_text(runtime, name, json!({ "label": label }))
        },
        "window.manage" => window_manage(runtime, &args),
        "screenshot.window" | "screenshot.thumbnail" => screenshot(runtime, name, &args),
        "webview.snapshot" => webview_snapshot(runtime, &args),
        "webview.find_element" => webview_find(runtime, &args),
        "webview.console_logs" | "webview.network_requests" => {
            let label = optional_string(&args, "windowId", 256)?;
            let since = optional_i64(&args, "since")?;
            live_text(runtime, name, json!({ "label": label, "since": since }))
        },
        "webview.click" => webview_click(runtime, &args),
        "webview.type" => webview_type(runtime, &args),
        "webview.fill" => webview_fill(runtime, &args),
        "webview.press_key" => webview_press_key(runtime, &args),
        "webview.scroll" => webview_scroll(runtime, &args),
        "webview.hover" => webview_hover(runtime, &args),
        "webview.drag" => webview_drag(runtime, &args),
        "webview.evaluate" => webview_evaluate(runtime, &args),
        "ipc.recent" => {
            let n = optional_u64(&args, "n", 50, 1, 2000)?;
            live_text(runtime, name, json!({ "n": n }))
        },
        "ipc.replay" => {
            let id = required_string(&args, "id", 1024)?;
            live_text(runtime, name, json!({ "id": id }))
        },
        "plugin.list" => plugin_list(runtime),
        "plugin.docs" => plugin_docs(runtime, &args),
        "plugin.command_catalog" => plugin_command_catalog(runtime, &args),
        "capability.check" => {
            let api = required_string(&args, "api", 1024)?;
            live_text(runtime, name, json!({ "api": api }))
        },
        "capability.suggest_grant" => capability_suggestion(&args),
        _ => bail!("unknown tool: {name}"),
    }
}

fn live_text(runtime: &mut Runtime, method: &str, params: Value) -> Result<Value> {
    json_text(runtime.request_live(method, params)?)
}

fn window_manage(runtime: &mut Runtime, args: &Map<String, Value>) -> Result<Value> {
    let label = required_string(args, "label", 256)?;
    let action = required_string(args, "action", 32)?;
    if !matches!(
        action,
        "move" | "resize" | "focus" | "minimize" | "maximize" | "close"
    ) {
        bail!("action must be move, resize, focus, minimize, maximize, or close");
    }
    let action_args = args.get("args").cloned().unwrap_or(Value::Null);
    live_text(
        runtime,
        "window.manage",
        json!({ "label": label, "action": action, "args": action_args }),
    )
}

fn screenshot(runtime: &mut Runtime, method: &str, args: &Map<String, Value>) -> Result<Value> {
    let label = optional_string(args, "label", 256)?;
    let params = label.map_or(Value::Null, |label| json!({ "label": label }));
    let shot = runtime.request_live(method, params)?;
    let data = shot
        .get("png_base64")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("bridge {method}: malformed screenshot response"))?;
    if data.len() > super::MAX_SCREENSHOT_BASE64_BYTES {
        bail!("bridge {method}: screenshot exceeds MCP size cap");
    }
    Ok(json!({
        "content": [{ "type": "image", "data": data, "mimeType": "image/png" }]
    }))
}

fn webview_snapshot(runtime: &mut Runtime, args: &Map<String, Value>) -> Result<Value> {
    let mode = required_string(args, "mode", 16)?;
    if !matches!(mode, "map" | "html" | "state") {
        bail!("mode must be map, html, or state");
    }
    let label = optional_string(args, "windowId", 256)?;
    live_text(
        runtime,
        "webview.snapshot",
        json!({ "mode": mode, "label": label }),
    )
}

fn webview_find(runtime: &mut Runtime, args: &Map<String, Value>) -> Result<Value> {
    let selector = required_string(args, "selector", 4096)?;
    let label = optional_string(args, "windowId", 256)?;
    live_text(
        runtime,
        "webview.find_element",
        json!({ "selector": selector, "label": label }),
    )
}

fn webview_click(runtime: &mut Runtime, args: &Map<String, Value>) -> Result<Value> {
    let label = optional_string(args, "windowId", 256)?;
    let target = if let Some(reference) = optional_string(args, "ref", 512)? {
        if reference.is_empty() {
            bail!("webview.click ref must not be empty");
        }
        json!({ "ref": reference })
    } else if let Some(coords) = args.get("coords") {
        validate_coords(coords)?;
        coords.clone()
    } else {
        bail!("webview.click requires ref or coords");
    };
    live_text(
        runtime,
        "webview.click",
        json!({ "label": label, "target": target }),
    )
}

fn webview_type(runtime: &mut Runtime, args: &Map<String, Value>) -> Result<Value> {
    let text = required_string(args, "text", 65_536)?;
    let label = optional_string(args, "windowId", 256)?;
    let target = optional_string(args, "ref", 512)?.map(|reference| json!({ "ref": reference }));
    live_text(
        runtime,
        "webview.type",
        json!({ "text": text, "label": label, "target": target }),
    )
}

fn webview_fill(runtime: &mut Runtime, args: &Map<String, Value>) -> Result<Value> {
    let fields = args
        .get("fields")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("fields must be an array"))?;
    if fields.is_empty() || fields.len() > 100 {
        bail!("fields must contain 1 to 100 items");
    }
    for field in fields {
        let field = field
            .as_object()
            .ok_or_else(|| anyhow!("each field must be an object"))?;
        required_string(field, "ref", 512)?;
        required_string_allow_empty(field, "value", 65_536)?;
    }
    let label = optional_string(args, "windowId", 256)?;
    live_text(
        runtime,
        "webview.fill",
        json!({ "fields": fields, "label": label }),
    )
}

fn webview_press_key(runtime: &mut Runtime, args: &Map<String, Value>) -> Result<Value> {
    let key = required_string(args, "key", 128)?;
    let label = optional_string(args, "windowId", 256)?;
    live_text(
        runtime,
        "webview.press_key",
        json!({ "key": key, "label": label }),
    )
}

fn webview_scroll(runtime: &mut Runtime, args: &Map<String, Value>) -> Result<Value> {
    let amount = args.get("amount").map_or(Ok(500.0), |value| {
        value
            .as_f64()
            .filter(|amount| amount.is_finite() && (0.0..=1_000_000.0).contains(amount))
            .ok_or_else(|| anyhow!("amount must be between 0 and 1000000"))
    })?;
    let target = if let Some(reference) = optional_string(args, "to_element", 512)? {
        json!({ "ref": reference })
    } else if args.get("to_top").and_then(Value::as_bool) == Some(true) {
        Value::String("top".into())
    } else if args.get("to_bottom").and_then(Value::as_bool) == Some(true) {
        Value::String("bottom".into())
    } else {
        match optional_string(args, "direction", 16)?.as_deref() {
            Some("up") => json!({ "dy": -amount }),
            Some("down") => json!({ "dy": amount }),
            Some("left") => json!({ "dx": -amount }),
            Some("right") => json!({ "dx": amount }),
            Some(_) => bail!("direction must be up, down, left, or right"),
            None => Value::Null,
        }
    };
    let label = optional_string(args, "windowId", 256)?;
    live_text(
        runtime,
        "webview.scroll",
        json!({ "label": label, "target": target, "opts": {} }),
    )
}

fn webview_hover(runtime: &mut Runtime, args: &Map<String, Value>) -> Result<Value> {
    let reference = required_string(args, "ref", 512)?;
    let label = optional_string(args, "windowId", 256)?;
    live_text(
        runtime,
        "webview.hover",
        json!({ "label": label, "target": { "ref": reference } }),
    )
}

fn webview_drag(runtime: &mut Runtime, args: &Map<String, Value>) -> Result<Value> {
    let start = drag_point(args.get("start"), "start")?;
    let end = drag_point(args.get("end"), "end")?;
    let label = optional_string(args, "windowId", 256)?;
    live_text(
        runtime,
        "webview.drag",
        json!({ "label": label, "start": start, "end": end }),
    )
}

fn webview_evaluate(runtime: &mut Runtime, args: &Map<String, Value>) -> Result<Value> {
    if !runtime.config.allow_eval {
        bail!(
            "webview.evaluate is disabled. Start the server with `--allow-eval` to enable JS evaluation."
        );
    }
    let script = required_string(args, "js", 262_144)?;
    let label = optional_string(args, "windowId", 256)?;
    live_text(
        runtime,
        "webview.evaluate",
        json!({ "script": script, "label": label }),
    )
}

fn plugin_list(runtime: &mut Runtime) -> Result<Value> {
    if let Some(result) = runtime.request_if_attached("plugin.list", Value::Null) {
        return json_text(result?);
    }
    let schemas = runtime.catalog.list_plugin_schemas()?;
    json_text(Value::Array(
        schemas
            .into_iter()
            .map(|schema| json!({ "name": schema.name, "summary": schema.summary }))
            .collect(),
    ))
}

fn plugin_docs(runtime: &mut Runtime, args: &Map<String, Value>) -> Result<Value> {
    let name = required_string(args, "name", 256)?.to_owned();
    let method = optional_string(args, "method", 256)?;
    let mut sections = Vec::new();
    if let Some(schema) = runtime.catalog.find_plugin_schema(&name)? {
        sections.push(json!({
            "title": format!("{} schema", schema.name),
            "schemaSource": schema.source,
        }));
    }
    let documents = match runtime.catalog.find_doc(&name)? {
        Some(document) => vec![document],
        None => runtime.catalog.search_docs(&name, 3)?,
    };
    sections.extend(documents.into_iter().map(|document| {
        json!({ "title": document.title, "source": document.slug, "body": document.body })
    }));
    if let Some(live) =
        runtime.request_if_attached("plugin.docs", json!({ "name": name, "method": method }))
    {
        match live {
            Ok(body) => {
                sections.push(json!({ "title": format!("{name} (live host)"), "body": body }));
            },
            Err(error) => sections.push(json!({
                "title": format!("{name} (live host) — error"),
                "body": error.to_string(),
            })),
        }
    }
    if sections.is_empty() {
        bail!("no docs found for plugin \"{name}\"");
    }
    json_text(json!({ "name": name, "method": method, "sections": sections }))
}

fn plugin_command_catalog(runtime: &mut Runtime, args: &Map<String, Value>) -> Result<Value> {
    let name = required_string(args, "name", 256)?;
    let schema = runtime
        .catalog
        .find_plugin_schema(name)?
        .ok_or_else(|| anyhow!("no plugin \"{name}\" in bundled catalog"))?;
    json_text(json!({ "name": schema.name, "schemaSource": schema.source }))
}

fn capability_suggestion(args: &Map<String, Value>) -> Result<Value> {
    let requested_api = required_string(args, "api", 1024)?;
    let command = normalize_capability_command(requested_api)?;
    let capability = json!({
        "identifier": "mcp-suggested",
        "windows": ["main"],
        "permissions": [{
            "identifier": "mcp-suggested-native-access",
            "commands": { "allow": [&command], "deny": [] }
        }]
    });
    let serialized =
        serde_json::to_string_pretty(&capability).context("encode suggested capability")?;
    json_text(json!({
        "requestedApi": requested_api,
        "command": command,
        "capability": capability,
        "example": format!("app.start({{ capabilities: [{serialized}] }});"),
        "doc": "tynd://docs/security",
        "notes": "Change the identifier and windows scope to match the app; grant only the required command."
    }))
}

fn normalize_capability_command(api: &str) -> Result<String> {
    match api.split_once(':') {
        Some((namespace, command)) if !namespace.is_empty() && !command.is_empty() => {
            Ok(api.to_owned())
        },
        _ => bail!("api must use namespace:command"),
    }
}

fn json_text(value: Value) -> Result<Value> {
    let text = match value {
        Value::String(text) => text,
        other => serde_json::to_string_pretty(&other).context("encode MCP tool result")?,
    };
    if text.len() > MAX_TOOL_TEXT_BYTES {
        bail!("MCP tool result exceeds cap ({MAX_TOOL_TEXT_BYTES} bytes)");
    }
    Ok(json!({ "content": [{ "type": "text", "text": text }] }))
}

fn error_result(message: &str) -> Value {
    json!({ "content": [{ "type": "text", "text": message }], "isError": true })
}

fn object_or_empty(value: Value) -> Result<Map<String, Value>> {
    match value {
        Value::Null => Ok(Map::new()),
        Value::Object(object) => Ok(object),
        _ => bail!("tool arguments must be a JSON object"),
    }
}

fn required_string<'a>(object: &'a Map<String, Value>, key: &str, max: usize) -> Result<&'a str> {
    let value = required_string_allow_empty(object, key, max)?;
    if value.is_empty() {
        bail!("{key} must not be empty");
    }
    Ok(value)
}

fn required_string_allow_empty<'a>(
    object: &'a Map<String, Value>,
    key: &str,
    max: usize,
) -> Result<&'a str> {
    let value = object
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("{key} must be a string"))?;
    if value.chars().count() > max {
        bail!("{key} exceeds {max} characters");
    }
    Ok(value)
}

fn optional_string(object: &Map<String, Value>, key: &str, max: usize) -> Result<Option<String>> {
    match object.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) if value.chars().count() <= max => Ok(Some(value.clone())),
        Some(Value::String(_)) => bail!("{key} exceeds {max} characters"),
        Some(_) => bail!("{key} must be a string"),
    }
}

fn optional_i64(object: &Map<String, Value>, key: &str) -> Result<Option<i64>> {
    match object.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_i64()
            .map(Some)
            .ok_or_else(|| anyhow!("{key} must be an integer")),
    }
}

fn optional_u64(
    object: &Map<String, Value>,
    key: &str,
    default: u64,
    min: u64,
    max: u64,
) -> Result<u64> {
    match object.get(key) {
        None | Some(Value::Null) => Ok(default),
        Some(value) => value
            .as_u64()
            .filter(|number| (min..=max).contains(number))
            .ok_or_else(|| anyhow!("{key} must be an integer between {min} and {max}")),
    }
}

fn validate_coords(value: &Value) -> Result<()> {
    let object = value
        .as_object()
        .ok_or_else(|| anyhow!("coordinates must be an object"))?;
    for key in ["x", "y"] {
        object
            .get(key)
            .and_then(Value::as_f64)
            .filter(|number| number.is_finite())
            .ok_or_else(|| anyhow!("coordinates.{key} must be finite"))?;
    }
    Ok(())
}

fn drag_point(value: Option<&Value>, key: &str) -> Result<Value> {
    match value {
        Some(Value::String(reference)) if !reference.is_empty() && reference.len() <= 512 => {
            Ok(json!({ "ref": reference }))
        },
        Some(value @ Value::Object(_)) => {
            validate_coords(value)?;
            Ok(value.clone())
        },
        _ => bail!("{key} must be a non-empty ref or coordinate object"),
    }
}

fn input_schema(name: &str) -> Value {
    let mut properties = Map::new();
    let mut required = Vec::new();
    if !app_window_input_schema(name, &mut properties, &mut required)
        && !webview_observation_input_schema(name, &mut properties, &mut required)
        && !webview_action_input_schema(name, &mut properties, &mut required)
    {
        diagnostic_input_schema(name, &mut properties, &mut required);
    }

    let mut schema = json!({
        "type": "object",
        "properties": properties,
        "required": required,
        "additionalProperties": false,
    });
    if name == "webview.click" {
        schema["anyOf"] = json!([
            { "required": ["ref"] },
            { "required": ["coords"] }
        ]);
    }
    schema
}

fn app_window_input_schema(
    name: &str,
    properties: &mut Map<String, Value>,
    required: &mut Vec<&'static str>,
) -> bool {
    match name {
        "app.logs.tail" => {
            properties.insert("n".into(), integer(1, 1000));
        },
        "app.launch" => {
            properties.insert("path".into(), string_schema());
            required.push("path");
        },
        "window.info" => {
            properties.insert("label".into(), string_schema());
            required.push("label");
        },
        "window.manage" => {
            properties.insert("label".into(), string_schema());
            properties.insert(
                "action".into(),
                json!({
                    "type": "string",
                    "enum": ["move", "resize", "focus", "minimize", "maximize", "close"]
                }),
            );
            properties.insert(
                "args".into(),
                json!({ "type": "object", "additionalProperties": true }),
            );
            required.extend(["label", "action"]);
        },
        "screenshot.window" | "screenshot.thumbnail" => {
            properties.insert("label".into(), string_schema());
        },
        _ => return false,
    }
    true
}

fn webview_observation_input_schema(
    name: &str,
    properties: &mut Map<String, Value>,
    required: &mut Vec<&'static str>,
) -> bool {
    match name {
        "webview.snapshot" => {
            properties.insert(
                "mode".into(),
                json!({ "type": "string", "enum": ["map", "html", "state"] }),
            );
            properties.insert("windowId".into(), string_schema());
            required.push("mode");
        },
        "webview.find_element" => {
            properties.insert("selector".into(), string_schema());
            properties.insert("windowId".into(), string_schema());
            required.push("selector");
        },
        "webview.console_logs" | "webview.network_requests" => {
            properties.insert("windowId".into(), string_schema());
            properties.insert("since".into(), json!({ "type": "integer" }));
        },
        "webview.click" => {
            properties.insert("ref".into(), non_empty_string_schema());
            properties.insert("coords".into(), coords_schema());
            properties.insert("windowId".into(), string_schema());
        },
        "webview.type" => {
            properties.insert("text".into(), string_schema());
            properties.insert("ref".into(), string_schema());
            properties.insert("windowId".into(), string_schema());
            required.push("text");
        },
        "webview.fill" => {
            properties.insert(
                "fields".into(),
                json!({
                    "type": "array", "minItems": 1, "maxItems": 100,
                    "items": { "type": "object", "properties": {
                        "ref": { "type": "string" }, "value": { "type": "string" }
                    }, "required": ["ref", "value"], "additionalProperties": false }
                }),
            );
            properties.insert("windowId".into(), string_schema());
            required.push("fields");
        },
        _ => return false,
    }
    true
}

fn webview_action_input_schema(
    name: &str,
    properties: &mut Map<String, Value>,
    required: &mut Vec<&'static str>,
) -> bool {
    match name {
        "webview.press_key" => {
            properties.insert("key".into(), string_schema());
            properties.insert("windowId".into(), string_schema());
            required.push("key");
        },
        "webview.scroll" => {
            properties.insert(
                "direction".into(),
                json!({ "type": "string", "enum": ["up", "down", "left", "right"] }),
            );
            properties.insert(
                "amount".into(),
                json!({ "type": "number", "minimum": 0, "maximum": 1_000_000 }),
            );
            properties.insert("to_element".into(), string_schema());
            properties.insert("to_top".into(), json!({ "type": "boolean" }));
            properties.insert("to_bottom".into(), json!({ "type": "boolean" }));
            properties.insert("windowId".into(), string_schema());
        },
        "webview.hover" => {
            properties.insert("ref".into(), string_schema());
            properties.insert("windowId".into(), string_schema());
            required.push("ref");
        },
        "webview.drag" => {
            let point = json!({ "oneOf": [{ "type": "string" }, coords_schema()] });
            properties.insert("start".into(), point.clone());
            properties.insert("end".into(), point);
            properties.insert("windowId".into(), string_schema());
            required.extend(["start", "end"]);
        },
        "webview.evaluate" => {
            properties.insert("js".into(), string_schema());
            properties.insert("windowId".into(), string_schema());
            required.push("js");
        },
        _ => return false,
    }
    true
}

fn diagnostic_input_schema(
    name: &str,
    properties: &mut Map<String, Value>,
    required: &mut Vec<&'static str>,
) {
    match name {
        "ipc.recent" => {
            properties.insert("n".into(), integer(1, 2000));
        },
        "ipc.replay" => {
            properties.insert("id".into(), string_schema());
            required.push("id");
        },
        "plugin.docs" => {
            properties.insert("name".into(), string_schema());
            properties.insert("method".into(), string_schema());
            required.push("name");
        },
        "plugin.command_catalog" => {
            properties.insert("name".into(), string_schema());
            required.push("name");
        },
        "capability.check" | "capability.suggest_grant" => {
            properties.insert("api".into(), string_schema());
            required.push("api");
        },
        _ => {},
    }
}

fn string_schema() -> Value {
    json!({ "type": "string" })
}

fn non_empty_string_schema() -> Value {
    json!({ "type": "string", "minLength": 1 })
}

fn integer(minimum: u64, maximum: u64) -> Value {
    json!({ "type": "integer", "minimum": minimum, "maximum": maximum })
}

fn coords_schema() -> Value {
    json!({
        "type": "object",
        "properties": { "x": { "type": "number" }, "y": { "type": "number" } },
        "required": ["x", "y"],
        "additionalProperties": false,
    })
}

#[cfg(test)]
mod tests {
    use super::normalize_capability_command;

    #[test]
    fn capability_command_requires_the_colon_form() {
        assert_eq!(
            normalize_capability_command("fs:readFile").expect("colon form"),
            "fs:readFile"
        );
        for rejected in ["fs.readFile", "fs:", ":readFile", "fs"] {
            assert!(
                normalize_capability_command(rejected).is_err(),
                "{rejected}"
            );
        }
    }
}
