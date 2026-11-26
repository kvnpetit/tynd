use std::fmt::Write as _;

use anyhow::{anyhow, bail, Result};
use serde_json::{json, Value};

use super::{
    catalog::{known_error_json, known_errors},
    runtime::Runtime,
};

pub(super) fn list_resources(runtime: &mut Runtime) -> Result<Value> {
    let mut resources = Vec::new();
    for document in runtime.catalog.list_docs()? {
        resources.push(json!({
            "uri": format!("tynd://docs/{}", document.slug),
            "name": document.slug,
            "title": document.title,
            "mimeType": "text/markdown",
        }));
    }
    for schema in runtime.catalog.list_plugin_schemas()? {
        resources.push(json!({
            "uri": format!("tynd://schema/plugin/{}", schema.name),
            "name": format!("{}.gen.ts", schema.name),
            "title": format!("{} plugin schema", schema.name),
            "mimeType": "text/x-typescript",
        }));
    }
    for error in known_errors() {
        resources.push(json!({
            "uri": format!("tynd://errors/{}", error.code),
            "name": error.code,
            "title": error.title,
            "mimeType": "application/json",
        }));
    }
    resources.push(json!({
        "uri": "tynd://logs/recent",
        "name": "recent-host-logs",
        "title": "Recent host logs",
        "mimeType": "application/json",
    }));
    resources.push(json!({
        "uri": "tynd://ipc/log",
        "name": "recent-ipc-frames",
        "title": "Recent IPC frames",
        "mimeType": "application/json",
    }));
    append_screenshot_resources(runtime, &mut resources);
    Ok(Value::Array(resources))
}

pub(super) fn list_templates() -> Value {
    json!([
        {
            "uriTemplate": "tynd://docs/{slug}",
            "name": "tynd-docs",
            "title": "Tynd framework docs (bundled)",
            "description": "Curated Tynd docs shipped with MCP; works offline.",
            "mimeType": "text/markdown"
        },
        {
            "uriTemplate": "tynd://schema/plugin/{name}",
            "name": "tynd-plugin-schema",
            "title": "Bundled plugin schemas",
            "description": "Generated TypeScript schema files for known plugins.",
            "mimeType": "text/x-typescript"
        },
        {
            "uriTemplate": "tynd://errors/{code}",
            "name": "tynd-known-errors",
            "title": "Known Tynd errors",
            "description": "Common errors with symptom, cause, and fix.",
            "mimeType": "application/json"
        },
        {
            "uriTemplate": "tynd://screenshot/{label}",
            "name": "tynd-screenshot",
            "title": "Window screenshots",
            "description": "PNG screenshot of a Tynd window. Live mode only.",
            "mimeType": "image/png"
        }
    ])
}

pub(super) fn read_resource(runtime: &mut Runtime, uri: &str) -> Result<Value> {
    if let Some(slug) = uri.strip_prefix("tynd://docs/") {
        let slug = percent_decode(slug);
        return Ok(match runtime.catalog.find_doc(&slug)? {
            Some(document) => text_content(uri, &document.body, "text/markdown"),
            None => text_content(
                uri,
                &format!("No bundled doc for \"{slug}\". List resources to discover slugs."),
                "text/plain",
            ),
        });
    }
    if let Some(name) = uri.strip_prefix("tynd://schema/plugin/") {
        let name = percent_decode(name);
        return Ok(match runtime.catalog.find_plugin_schema(&name)? {
            Some(schema) => text_content(uri, &schema.source, "text/x-typescript"),
            None => text_content(
                uri,
                &format!("No bundled schema for plugin \"{name}\"."),
                "text/plain",
            ),
        });
    }
    if let Some(code) = uri.strip_prefix("tynd://errors/") {
        let code = percent_decode(code);
        return Ok(match super::catalog::find_known_error(&code) {
            Some(error) => json_content(uri, known_error_json(error))?,
            None => text_content(
                uri,
                &format!("No catalog entry for code \"{code}\"."),
                "text/plain",
            ),
        });
    }
    match uri {
        "tynd://logs/recent" => read_live_json(runtime, uri, "app.logs.tail", json!({ "n": 200 })),
        "tynd://ipc/log" => read_live_json(runtime, uri, "ipc.recent", json!({ "n": 200 })),
        _ if uri.starts_with("tynd://screenshot/") => read_screenshot(runtime, uri),
        _ => Err(anyhow!("unknown resource URI: {uri}")),
    }
}

fn append_screenshot_resources(runtime: &mut Runtime, resources: &mut Vec<Value>) {
    let Some(result) = runtime.request_if_attached("window.list", Value::Null) else {
        return;
    };
    match result {
        Ok(Value::Array(windows)) => {
            for window in windows {
                let Some(label) = window.get("label").and_then(Value::as_str) else {
                    continue;
                };
                let title = window.get("title").and_then(Value::as_str).unwrap_or(label);
                resources.push(json!({
                    "uri": format!("tynd://screenshot/{}", percent_encode(label)),
                    "name": format!("{label}.png"),
                    "title": title,
                    "mimeType": "image/png",
                }));
            }
        },
        Ok(_) | Err(_) => resources.push(json!({
            "uri": "tynd://screenshot/__error__",
            "name": "bridge-error.txt",
            "title": "Bridge unavailable",
            "mimeType": "text/plain",
        })),
    }
}

fn read_live_json(runtime: &mut Runtime, uri: &str, method: &str, params: Value) -> Result<Value> {
    match runtime.request_if_attached(method, params) {
        None => Ok(text_content(
            uri,
            "No attached tynd app — start `tynd dev` first.",
            "text/plain",
        )),
        Some(Ok(value)) => json_content(uri, value),
        Some(Err(error)) => json_content(uri, json!({ "error": error.to_string(), "frames": [] })),
    }
}

fn read_screenshot(runtime: &mut Runtime, uri: &str) -> Result<Value> {
    let label = percent_decode(uri.trim_start_matches("tynd://screenshot/"));
    if label == "__error__" {
        return Ok(text_content(
            uri,
            "Bridge was unavailable when windows were listed; list resources again to retry.",
            "text/plain",
        ));
    }
    let Some(result) = runtime.request_if_attached("screenshot.window", json!({ "label": label }))
    else {
        return Ok(text_content(
            uri,
            "No attached tynd app — start `tynd dev` first.",
            "text/plain",
        ));
    };
    let shot = result?;
    let blob = shot
        .get("png_base64")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("bridge screenshot.window: malformed response"))?;
    if blob.len() > super::MAX_SCREENSHOT_BASE64_BYTES {
        bail!("bridge screenshot.window: screenshot exceeds MCP size cap");
    }
    Ok(json!({
        "contents": [{ "uri": uri, "mimeType": "image/png", "blob": blob }]
    }))
}

fn text_content(uri: &str, text: &str, mime_type: &str) -> Value {
    json!({ "contents": [{ "uri": uri, "mimeType": mime_type, "text": text }] })
}

fn json_content(uri: &str, value: Value) -> Result<Value> {
    Ok(text_content(
        uri,
        &serde_json::to_string_pretty(&value)?,
        "application/json",
    ))
}

fn percent_encode(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            output.push(char::from(byte));
        } else {
            write!(output, "%{byte:02X}").expect("writing to a string cannot fail");
        }
    }
    output
}

fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            if let (Some(high), Some(low)) = (hex(bytes[index + 1]), hex(bytes[index + 2])) {
                output.push((high << 4) | low);
                index += 3;
                continue;
            }
        }
        output.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&output).into_owned()
}

fn hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}
