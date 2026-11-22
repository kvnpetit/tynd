//! `plugin.*` debug methods — registry inspection.

use serde_json::{json, Value};

use super::CallOutcome;

fn plugin_count() -> usize {
    #[cfg(feature = "plugins")]
    {
        crate::plugins::count()
    }
    #[cfg(not(feature = "plugins"))]
    {
        0
    }
}

pub(crate) fn list(_params: Value) -> CallOutcome {
    #[cfg(feature = "plugins")]
    {
        CallOutcome::Result(Value::Array(crate::plugins::introspect()))
    }
    #[cfg(not(feature = "plugins"))]
    {
        CallOutcome::Result(Value::Array(Vec::new()))
    }
}

pub(crate) fn docs(params: Value) -> CallOutcome {
    let name = params.get("name").and_then(Value::as_str).unwrap_or("");
    if name.is_empty() {
        return CallOutcome::Error(tynd_debug_protocol::protocol::RpcError::invalid_params(
            "missing 'name'",
        ));
    }
    CallOutcome::Result(json!({
        "name": name,
        "method": params.get("method").cloned().unwrap_or(Value::Null),
        "loadedPluginCount": plugin_count(),
        "note": "authoritative plugin documentation and generated command schemas are provided by @tynd/mcp",
    }))
}

pub(crate) fn command_catalog(params: Value) -> CallOutcome {
    // The macro-driven dispatch tables don't expose a runtime catalog
    // today — surface the count + a hint pointing at `crate::security::acl` for
    // command-name introspection.
    CallOutcome::Result(json!({
        "name": params.get("name").cloned().unwrap_or(Value::Null),
        "commands": [],
        "loadedPluginCount": plugin_count(),
        "authoritative": false,
        "note": "authoritative generated command schemas are provided by @tynd/mcp",
    }))
}
