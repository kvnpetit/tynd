//! `logs.subscribe` — stream `tracing` events to the attached debugger.

use serde_json::{json, Value};
use std::sync::atomic::Ordering;
use tynd_debug_protocol::{log_tap, SubscriptionHandle};

use super::CallOutcome;

pub(crate) fn subscribe(_params: Value) -> CallOutcome {
    let Some(rx) = log_tap::subscribe() else {
        return CallOutcome::Error(tynd_debug_protocol::protocol::RpcError::internal_error(
            "log tap not initialized",
        ));
    };
    let handle = SubscriptionHandle::new(
        "log.frame",
        Box::new(move |emit, cancel| {
            while !cancel.load(Ordering::Acquire) {
                match rx.recv_timeout(std::time::Duration::from_millis(250)) {
                    Ok(frame) if !emit(serde_json::to_value(&frame).unwrap_or(Value::Null)) => {
                        break;
                    },
                    Ok(_) | Err(crossbeam_channel::RecvTimeoutError::Timeout) => {},
                    Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
                }
            }
        }),
    );
    CallOutcome::Subscription(handle)
}

pub(crate) fn tail(params: Value) -> CallOutcome {
    let n = params
        .get("n")
        .and_then(Value::as_u64)
        .unwrap_or(100)
        .min(1_000) as usize;
    CallOutcome::Result(json!(log_tap::recent(n)))
}
