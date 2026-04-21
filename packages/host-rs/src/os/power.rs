//! User idle time + system power queries.
//!
//! Today: idle-seconds only (cross-platform, zero-event). Sleep / wake / lock
//! events need per-OS subscriptions (WM_POWERBROADCAST, IOKit, logind) and
//! aren't wired yet — apps that need them should install a watchdog in their
//! backend and poll `getIdleTime` for now.

use serde_json::{json, Value};

pub fn dispatch(method: &str, _args: &Value) -> Result<Value, String> {
    match method {
        "getIdleTime" => Ok(json!({ "seconds": idle_seconds()? })),
        _ => Err(format!("power.{method}: unknown method")),
    }
}

fn idle_seconds() -> Result<u64, String> {
    user_idle::UserIdle::get_time()
        .map(|t| t.as_seconds())
        .map_err(|e| format!("power.getIdleTime: {e}"))
}
