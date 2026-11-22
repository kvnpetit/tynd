use std::io::BufRead;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tynd_host::tynd_info;
use tynd_protocol::{
    runtime::{BackendCall, BackendConfig, MenuItemDef, TrayConfig, WINDOW_EVENT_FN},
    window::WindowConfig,
    MAX_IPC_FRAME_BYTES,
};

/// Outbound wire frame to Bun's stdin. `serde(tag = "type")` reproduces the
/// JS-side contract `{ "type": "...", ... }`.
#[derive(Debug, Serialize)]
#[serde(tag = "type")]
enum WireMsg<'a> {
    #[serde(rename = "tynd:ready")]
    Ready,
    #[serde(rename = "tynd:close")]
    Close,
    #[serde(rename = "tynd:window")]
    Window { event: &'a str, label: &'a str },
    #[serde(rename = "call")]
    Call {
        id: &'a str,
        #[serde(rename = "fn")]
        fn_name: &'a str,
        args: &'a [Value],
    },
    #[serde(rename = "cancel")]
    Cancel { id: &'a str },
    #[serde(rename = "ack")]
    Ack { id: &'a str, n: u32 },
    #[serde(rename = "host_result")]
    HostResult {
        id: &'a str,
        ok: bool,
        value: &'a Value,
    },
}

/// Strip the trailing `\n` / `\r\n` from a `read_until(b'\n', ...)` buffer.
#[inline]
pub(super) fn strip_trailing_eol(buf: &[u8]) -> &[u8] {
    let mut end = buf.len();
    if end > 0 && buf[end - 1] == b'\n' {
        end -= 1;
    }
    if end > 0 && buf[end - 1] == b'\r' {
        end -= 1;
    }
    &buf[..end]
}

/// JSON decode helper for Bun stdout frames.
pub(super) fn parse_bun_msg(s: &str) -> Result<BunMsg, String> {
    serde_json::from_str::<BunMsg>(s).map_err(|e| e.to_string())
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub(super) enum BunMsg {
    #[serde(rename = "tynd:config")]
    Config {
        window: Box<Option<WindowConfig>>,
        #[serde(rename = "devUrl")]
        dev_url: Option<String>,
        #[serde(rename = "frontendDir")]
        frontend_dir: Option<String>,
        #[serde(default)]
        menu: Vec<MenuItemDef>,
        tray: Option<TrayConfig>,
        security: Option<Value>,
        capabilities: Option<Value>,
        #[serde(default, rename = "quitOnLastWindowClosed")]
        quit_on_last_window_closed: bool,
    },
    #[serde(rename = "return")]
    Return {
        id: String,
        ok: bool,
        /// Devalue-encoded payload (success only). On error only `error` is set.
        #[serde(default)]
        v: Option<String>,
        error: Option<String>,
    },
    #[serde(rename = "yield")]
    Yield { id: String, v: String },
    #[serde(rename = "event")]
    Event {
        name: String,
        /// Devalue-encoded payload — routed verbatim to the webview shim.
        p: String,
        #[serde(default)]
        to: Option<String>,
    },
    #[serde(rename = "evalInFrontend")]
    EvalInFrontend {
        script: String,
        #[serde(default)]
        to: Option<String>,
    },
    #[serde(rename = "host_call")]
    HostCall {
        id: String,
        api: String,
        method: String,
        #[serde(default)]
        args: Value,
    },
}

/// Extract the compact shim-generated call id without parsing the full frame.
/// This keeps the forwarder hot path allocation-free until it needs to report
/// a rejected raw frame back to the originating WebView.
pub(super) fn extract_raw_call_id(body: &str) -> Option<&str> {
    let after = body.split_once(r#""id":""#)?.1;
    let end = after.find('"')?;
    let id = &after[..end];
    (!id.is_empty() && id.len() <= 128).then_some(id)
}

/// Append the `\n`-terminated JSON frame of `call` to `out`.
pub(super) fn push_call_line(out: &mut Vec<u8>, call: BackendCall) -> Result<(), String> {
    match call {
        // `Raw` is a pre-encoded JSON line from the webview shim.
        BackendCall::Raw(s) => {
            let mut bytes = s.into_bytes();
            if bytes.len() > MAX_IPC_FRAME_BYTES {
                return Err(format!(
                    "raw JSON frame exceeds {MAX_IPC_FRAME_BYTES} byte limit"
                ));
            }
            serde_json::from_slice::<Value>(&bytes)
                .map_err(|error| format!("Raw frame is not valid singular JSON: {error}"))?;
            bytes.push(b'\n');
            out.extend_from_slice(&bytes);
        },
        BackendCall::Typed { id, fn_name, args } => {
            let msg = match fn_name.as_str() {
                WINDOW_EVENT_FN => match args.as_slice() {
                    [Value::String(event), Value::String(label)]
                        if matches!(event.as_str(), "created" | "closed") =>
                    {
                        WireMsg::Window { event, label }
                    },
                    _ => return Err(format!("malformed {WINDOW_EVENT_FN} call: {args:?}")),
                },
                "__tynd_on_ready__" => WireMsg::Ready,
                "__tynd_on_close__" => WireMsg::Close,
                _ => WireMsg::Call {
                    id: &id,
                    fn_name: &fn_name,
                    args: &args,
                },
            };
            encode_wire_msg(out, &msg)?;
        },
        BackendCall::Cancel { id } => {
            encode_wire_msg(out, &WireMsg::Cancel { id: &id })?;
        },
        BackendCall::Ack { id, n } => {
            encode_wire_msg(out, &WireMsg::Ack { id: &id, n })?;
        },
        BackendCall::HostResult { id, ok, value } => {
            encode_wire_msg(
                out,
                &WireMsg::HostResult {
                    id: &id,
                    ok,
                    value: &value,
                },
            )?;
        },
    }
    Ok(())
}

/// Append one `WireMsg` to `out` as a `\n`-terminated JSON line. A serialize
/// failure or an oversized frame is returned as an error and nothing is
/// appended.
fn encode_wire_msg(out: &mut Vec<u8>, msg: &WireMsg<'_>) -> Result<(), String> {
    let mut s = serde_json::to_string(msg)
        .map_err(|e| format!("failed to serialize WireMsg as JSON: {e}"))?;
    if s.len() > MAX_IPC_FRAME_BYTES {
        return Err(format!(
            "JSON frame exceeds {MAX_IPC_FRAME_BYTES} byte limit"
        ));
    }
    s.push('\n');
    out.extend_from_slice(s.as_bytes());
    Ok(())
}

pub(super) fn read_bounded_json_line<R: BufRead>(
    reader: &mut R,
    output: &mut Vec<u8>,
) -> std::io::Result<usize> {
    read_bounded_line(reader, output, MAX_IPC_FRAME_BYTES)
}

pub(super) fn read_bounded_line<R: BufRead>(
    reader: &mut R,
    output: &mut Vec<u8>,
    max_bytes: usize,
) -> std::io::Result<usize> {
    output.clear();
    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            return Ok(output.len());
        }
        let newline = available.iter().position(|byte| *byte == b'\n');
        let take = newline.map_or(available.len(), |index| index + 1);
        let payload_bytes = if newline.is_some() { take - 1 } else { take };
        if output
            .len()
            .checked_add(payload_bytes)
            .is_none_or(|len| len > max_bytes)
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("JSON frame exceeds {max_bytes} byte limit"),
            ));
        }
        output.extend_from_slice(&available[..take]);
        reader.consume(take);
        if newline.is_some() {
            return Ok(output.len());
        }
    }
}

pub(super) fn read_config<R: BufRead>(reader: &mut R) -> Result<BackendConfig, String> {
    let msg = read_first_bun_msg(reader)?;
    match msg {
        BunMsg::Config {
            window,
            dev_url,
            frontend_dir,
            menu,
            tray,
            security,
            capabilities,
            quit_on_last_window_closed,
        } => {
            let icon_path = std::env::var("TYND_ICON_PATH").ok();
            Ok(BackendConfig {
                window: (*window).unwrap_or_default(),
                dev_url,
                frontend_dir,
                icon_path,
                menu,
                tray,
                security,
                capabilities,
                quit_on_last_window_closed,
                ..Default::default()
            })
        },
        other => Err(format!("Expected tynd:config, got: {other:?}")),
    }
}

/// Read until we land on a `BunMsg`. Log lines (no leading `{`) seen before
/// it are re-emitted to stderr.
pub(super) fn read_first_bun_msg<R: BufRead>(reader: &mut R) -> Result<BunMsg, String> {
    loop {
        let mut line = Vec::new();
        match read_bounded_json_line(reader, &mut line) {
            Ok(0) => {
                return Err("Runtime exited before sending config (is app.start() called?)".into());
            },
            Err(error) => {
                return Err(format!("Failed reading runtime config frame: {error}"));
            },
            Ok(_) => {},
        }
        let line = std::str::from_utf8(&line)
            .map_err(|error| format!("Runtime emitted non-UTF-8 config/log data: {error}"))?;
        let trimmed = line.trim();
        if !trimmed.starts_with('{') {
            tynd_info!("[bun] {trimmed}");
            continue;
        }
        return parse_bun_msg(trimmed)
            .map_err(|e| format!("Failed to parse tynd:config: {e}\nLine: {trimmed}"));
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{json, Value};
    use tynd_protocol::runtime::{BackendCall, WindowEventKind};

    use super::push_call_line;

    fn encode_json(call: BackendCall) -> Result<Value, String> {
        let mut out = Vec::new();
        push_call_line(&mut out, call)?;
        if out.last() != Some(&b'\n') {
            return Err("frame is not newline-terminated".to_string());
        }
        serde_json::from_slice(&out).map_err(|error| format!("invalid json line: {error}"))
    }

    #[test]
    fn window_created_and_closed_frames() {
        for (kind, event) in [
            (WindowEventKind::Created, "created"),
            (WindowEventKind::Closed, "closed"),
        ] {
            let frame = encode_json(BackendCall::window_event(kind, "settings")).unwrap();
            assert_eq!(
                frame,
                json!({ "type": "tynd:window", "event": event, "label": "settings" })
            );
        }
    }

    #[test]
    fn window_event_label_is_json_escaped() {
        let frame = encode_json(BackendCall::window_event(
            WindowEventKind::Created,
            "a\"b\\c",
        ))
        .unwrap();
        assert_eq!(frame["label"], json!("a\"b\\c"));
    }

    #[test]
    fn malformed_window_event_is_rejected() {
        let bad_event = BackendCall::call(
            super::WINDOW_EVENT_FN,
            super::WINDOW_EVENT_FN,
            vec![json!("exploded"), json!("main")],
        );
        assert!(encode_json(bad_event).is_err());
        let bad_arity = BackendCall::call(
            super::WINDOW_EVENT_FN,
            super::WINDOW_EVENT_FN,
            vec![json!("created")],
        );
        assert!(encode_json(bad_arity).is_err());
    }

    #[test]
    fn existing_lifecycle_frames_unchanged() {
        assert_eq!(
            encode_json(BackendCall::lifecycle("on_ready")).unwrap(),
            json!({ "type": "tynd:ready" })
        );
        assert_eq!(
            encode_json(BackendCall::lifecycle("on_close")).unwrap(),
            json!({ "type": "tynd:close" })
        );
    }
}
