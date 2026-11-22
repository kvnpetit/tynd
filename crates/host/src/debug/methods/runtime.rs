//! `runtime.snapshot` — host IPC and plugin facts for the Bun runtime.

use serde_json::{json, Value};

use super::CallOutcome;

pub(crate) fn snapshot(_params: Value) -> CallOutcome {
    #[cfg(feature = "plugins")]
    let plugin_count = crate::plugins::count();
    #[cfg(not(feature = "plugins"))]
    let plugin_count = 0_usize;
    let ipc_recent = tynd_debug_protocol::ipc_tap::recent(usize::MAX).len();
    CallOutcome::Result(json!({
        "mode": "bun",
        "pluginCount": plugin_count,
        "ipcRingDepth": ipc_recent,
        "pid": std::process::id(),
    }))
}
