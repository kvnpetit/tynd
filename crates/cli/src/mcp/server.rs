use std::io::{self, BufRead, BufReader, BufWriter, Write};

use anyhow::{Context, Result};
use serde_json::{json, Map, Value};

use super::{prompts, resources, runtime::Runtime, tools};

const MAX_STDIO_MESSAGE_BYTES: usize = 16 * 1024 * 1024;
const PROTOCOL_VERSION: &str = "2025-06-18";

pub(super) fn serve(runtime: &mut Runtime) -> Result<()> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut reader = BufReader::new(stdin.lock());
    let mut writer = BufWriter::new(stdout.lock());
    let mut buffer = Vec::new();
    let mut shutdown = false;

    loop {
        let Some(oversized) =
            read_bounded_line(&mut reader, &mut buffer).context("read MCP stdio")?
        else {
            break;
        };
        let outcome = if oversized {
            Outcome::Respond(error(Value::Null, -32_600, "request exceeds MCP size cap"))
        } else if buffer.is_empty() {
            Outcome::Ignore
        } else {
            handle_message(runtime, &buffer, &mut shutdown)
        };
        match outcome {
            Outcome::Respond(response) => write_response(&mut writer, response)?,
            Outcome::Ignore => {},
            Outcome::Exit => break,
        }
    }
    Ok(())
}

/// What the stdio loop does with one incoming line.
enum Outcome {
    Respond(Value),
    Ignore,
    Exit,
}

/// Handle one JSON-RPC message. Notifications (no `id`) never get a response;
/// the `exit` notification stops the server. After `shutdown`, every request
/// is refused.
fn handle_message(runtime: &mut Runtime, raw: &[u8], shutdown: &mut bool) -> Outcome {
    let message: Value = match serde_json::from_slice(raw) {
        Ok(message) => message,
        Err(parse_error) => {
            return Outcome::Respond(error(
                Value::Null,
                -32_700,
                &format!("parse error: {parse_error}"),
            ));
        },
    };
    if message.is_array() {
        return Outcome::Respond(error(
            Value::Null,
            -32_600,
            "batch requests are not supported",
        ));
    }
    let Some(object) = message.as_object() else {
        return Outcome::Respond(error(Value::Null, -32_600, "invalid request"));
    };
    let id = object.get("id").cloned();
    let Some(method) = object.get("method").and_then(Value::as_str) else {
        return match id {
            Some(id) => Outcome::Respond(error(id, -32_600, "method missing")),
            None => Outcome::Ignore,
        };
    };
    let Some(id) = id else {
        return if method == "exit" {
            Outcome::Exit
        } else {
            Outcome::Ignore
        };
    };
    if *shutdown {
        return Outcome::Respond(error(
            id,
            -32_600,
            "server is shutting down; send the exit notification",
        ));
    }
    let params = object.get("params").cloned().unwrap_or(Value::Null);
    let response = match dispatch(runtime, method, params) {
        Ok(result) => success(id, result),
        Err(DispatchError::Invalid(message)) => error(id, -32_602, &message),
        Err(DispatchError::Unknown) => error(id, -32_601, "method not found"),
        Err(DispatchError::Internal(message)) => error(id, -32_603, &message),
    };
    if method == "shutdown" {
        *shutdown = true;
    }
    Outcome::Respond(response)
}

#[derive(Debug)]
enum DispatchError {
    Invalid(String),
    Unknown,
    Internal(String),
}

fn dispatch(runtime: &mut Runtime, method: &str, params: Value) -> Result<Value, DispatchError> {
    match method {
        // Only one protocol version is implemented, so it is the answer whatever
        // `protocolVersion` the client requested.
        "initialize" => Ok(json!({
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": {
                "tools": { "listChanged": false },
                "resources": { "listChanged": false, "subscribe": false },
                "prompts": { "listChanged": false }
            },
            "serverInfo": {
                "name": "tynd-mcp",
                "version": env!("CARGO_PKG_VERSION")
            },
            "instructions": "Use static Tynd docs and plugin schemas offline; live tools attach automatically to `tynd dev`."
        })),
        "ping" => Ok(Value::Object(Map::default())),
        "shutdown" => Ok(Value::Null),
        "tools/list" => Ok(json!({ "tools": tools::list_tools() })),
        "tools/call" => {
            let name = params
                .get("name")
                .and_then(Value::as_str)
                .ok_or_else(|| DispatchError::Invalid("tools/call requires name".into()))?;
            let arguments = params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            Ok(tools::call_tool(runtime, name, arguments))
        },
        "resources/list" => resources::list_resources(runtime)
            .map(|resources| json!({ "resources": resources }))
            .map_err(internal),
        "resources/templates/list" => {
            Ok(json!({ "resourceTemplates": resources::list_templates() }))
        },
        "resources/read" => {
            let uri = params
                .get("uri")
                .and_then(Value::as_str)
                .ok_or_else(|| DispatchError::Invalid("resources/read requires uri".into()))?;
            resources::read_resource(runtime, uri).map_err(|error| {
                DispatchError::Invalid(super::bridge::sanitize_text(&error.to_string()))
            })
        },
        "prompts/list" => Ok(json!({ "prompts": prompts::list_prompts() })),
        "prompts/get" => {
            let name = params
                .get("name")
                .and_then(Value::as_str)
                .ok_or_else(|| DispatchError::Invalid("prompts/get requires name".into()))?;
            prompts::get_prompt(runtime, name).map_err(|error| {
                DispatchError::Invalid(super::bridge::sanitize_text(&error.to_string()))
            })
        },
        "completion/complete" => Ok(json!({
            "completion": { "values": [], "total": 0, "hasMore": false }
        })),
        _ => Err(DispatchError::Unknown),
    }
}

fn internal(error: anyhow::Error) -> DispatchError {
    DispatchError::Internal(super::bridge::sanitize_text(&error.to_string()))
}

fn success(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn error(id: Value, code: i64, message: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": super::bridge::sanitize_text(message) }
    })
}

fn read_bounded_line(reader: &mut impl BufRead, buffer: &mut Vec<u8>) -> io::Result<Option<bool>> {
    buffer.clear();
    let mut saw_input = false;
    let mut truncated = false;

    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            break;
        }
        saw_input = true;
        let newline = available.iter().position(|byte| *byte == b'\n');
        let consumed = newline.map_or(available.len(), |index| index + 1);
        let remaining = (MAX_STDIO_MESSAGE_BYTES + 2).saturating_sub(buffer.len());
        let copied = consumed.min(remaining);
        buffer.extend_from_slice(&available[..copied]);
        truncated |= copied < consumed;
        reader.consume(consumed);
        if newline.is_some() {
            break;
        }
    }

    if !saw_input {
        return Ok(None);
    }
    while matches!(buffer.last(), Some(b'\n' | b'\r')) {
        buffer.pop();
    }
    Ok(Some(truncated || buffer.len() > MAX_STDIO_MESSAGE_BYTES))
}

fn write_response(writer: &mut impl Write, response: Value) -> Result<()> {
    serde_json::to_writer(&mut *writer, &response).context("encode MCP response")?;
    writer.write_all(b"\n").context("write MCP response")?;
    writer.flush().context("flush MCP response")?;
    Ok(())
}
