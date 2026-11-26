//! `ipc.*` debug methods — recent envelope ring, failed-call view, live tap.

use serde_json::{json, Value};
use std::sync::atomic::Ordering;
use tynd_debug_protocol::{ipc_tap, SubscriptionHandle};

use super::CallOutcome;

pub(crate) fn subscribe(_params: Value) -> CallOutcome {
    let Some(rx) = ipc_tap::subscribe() else {
        return CallOutcome::Error(tynd_debug_protocol::protocol::RpcError::internal_error(
            "ipc tap not initialized",
        ));
    };
    let handle = SubscriptionHandle::new(
        "ipc.frame",
        Box::new(move |emit, cancel| {
            while !cancel.load(Ordering::Acquire) {
                match rx.recv_timeout(std::time::Duration::from_millis(250)) {
                    Ok(frame) => {
                        if !emit(wire_frame(frame)) {
                            break;
                        }
                    },
                    Err(crossbeam_channel::RecvTimeoutError::Timeout) => {},
                    Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
                }
            }
        }),
    );
    CallOutcome::Subscription(handle)
}

pub(crate) fn replay(params: Value) -> CallOutcome {
    let Some(id) = params.get("id").and_then(Value::as_str) else {
        return CallOutcome::Error(tynd_debug_protocol::protocol::RpcError::invalid_params(
            "missing 'id'",
        ));
    };
    match crate::debug::replay_ipc(id) {
        Ok(value) => CallOutcome::Result(value),
        Err(error) => CallOutcome::Error(tynd_debug_protocol::protocol::RpcError::internal_error(
            error,
        )),
    }
}

pub(crate) fn recent(params: Value) -> CallOutcome {
    let n = params
        .get("n")
        .and_then(Value::as_u64)
        .unwrap_or(64)
        .min(2_000) as usize;
    let frames: Vec<Value> = ipc_tap::recent(n).into_iter().map(wire_frame).collect();
    CallOutcome::Result(Value::Array(frames))
}

pub(crate) fn failed(params: Value) -> CallOutcome {
    let n = params
        .get("n")
        .and_then(Value::as_u64)
        .unwrap_or(64)
        .min(2_000) as usize;
    let all = ipc_tap::recent(n.saturating_mul(4));
    let failed: Vec<Value> = all
        .into_iter()
        .filter(|frame| frame.result.get("error").is_some())
        .take(n)
        .map(wire_frame)
        .collect();
    CallOutcome::Result(Value::Array(failed))
}

fn wire_frame(frame: tynd_debug_protocol::IpcFrame) -> Value {
    let id = frame
        .call_id
        .clone()
        .unwrap_or_else(|| format!("{}-{}", frame.ts, frame.direction));
    let direction = match frame.direction {
        "in" => "request",
        "out" => "response",
        _ => "event",
    };
    let method = frame.method.clone();
    let channel = method
        .as_deref()
        .and_then(|name| name.strip_prefix("$plugin$"))
        .and_then(|name| name.split('$').next())
        .unwrap_or("runtime");
    let payload = if frame.direction == "in" {
        frame.args
    } else {
        frame.result.clone()
    };
    let error = frame.result.get("error").cloned();
    let mut value = json!({
        "id": id,
        "ts": frame.ts,
        "direction": direction,
        "channel": channel,
        "payload": payload,
    });
    if let Some(object) = value.as_object_mut() {
        if let Some(method) = method {
            object.insert("fn".into(), Value::String(method));
        }
        if let Some(error) = error {
            object.insert("error".into(), error);
        }
        if let Some(duration) = frame.duration_ms {
            object.insert("duration_ms".into(), Value::from(duration));
        }
    }
    value
}
