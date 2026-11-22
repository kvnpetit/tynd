use anyhow::{anyhow, Result};
use serde_json::{json, Value};

use super::runtime::Runtime;

const PROMPTS: [(&str, &str, &str); 6] = [
    (
        "setup",
        "Verify Tynd MCP setup",
        "Verify zero-config discovery and current attach status.",
    ),
    (
        "debug-ipc-error",
        "Debug a recent IPC error",
        "Diagnose the most recent failed IPC frame and propose a concrete fix.",
    ),
    (
        "fix-webview-errors",
        "Fix webview console / network errors",
        "Triage captured webview errors and failed requests.",
    ),
    (
        "fix-permission-denied",
        "Fix a PERMISSION_DENIED IPC error",
        "Grant the missing capability and verify it live.",
    ),
    (
        "visual-regression",
        "Visual regression check",
        "Capture every window and report meaningful visual deltas.",
    ),
    (
        "explain-stack",
        "Explain a Tynd stack trace",
        "Find and explain the latest host stack trace.",
    ),
];

pub(super) fn list_prompts() -> Value {
    Value::Array(
        PROMPTS
            .iter()
            .map(|(name, title, description)| {
                json!({ "name": name, "title": title, "description": description, "arguments": [] })
            })
            .collect(),
    )
}

pub(super) fn get_prompt(runtime: &mut Runtime, name: &str) -> Result<Value> {
    let text = match name {
        "setup" => setup(runtime),
        "debug-ipc-error" => DEBUG_IPC_ERROR.to_owned(),
        "fix-webview-errors" => FIX_WEBVIEW_ERRORS.to_owned(),
        "fix-permission-denied" => FIX_PERMISSION_DENIED.to_owned(),
        "visual-regression" => VISUAL_REGRESSION.to_owned(),
        "explain-stack" => EXPLAIN_STACK.to_owned(),
        _ => return Err(anyhow!("unknown prompt: {name}")),
    };
    let description = PROMPTS
        .iter()
        .find(|prompt| prompt.0 == name)
        .map(|prompt| prompt.2)
        .unwrap_or_default();
    Ok(json!({
        "description": description,
        "messages": [{ "role": "user", "content": { "type": "text", "text": text } }]
    }))
}

fn setup(runtime: &mut Runtime) -> String {
    let status = runtime.discovery_status();
    let discovery = if status.get("ok").and_then(Value::as_bool) == Some(true) {
        format!(
            "Discovery scan: found {} live tynd dev app(s).",
            status.get("found").and_then(Value::as_u64).unwrap_or(0)
        )
    } else {
        "Discovery scan: failed (see MCP server stderr).".into()
    };
    let attached = runtime.attached_entry().map_or_else(
        || {
            "Not attached. Static tools (plugin.list, plugin.docs, capability.suggest_grant, framework docs) still work."
                .to_owned()
        },
        |entry| {
            format!(
                "Attached: app_id={}, pid={}, started_at_ms={}.",
                entry.app_id, entry.pid, entry.started_at_ms
            )
        },
    );
    format!(
        "Tynd MCP {} uses zero-config discovery. There is no manifest to add to your project.\n\nCurrent status:\n{}\n{}\n\nNext steps:\n1. If no live app shows up, run `tynd dev` in your Tynd project.\n2. Wait for the next discovery scan.\n3. Use `app.info` once attached to confirm the host responds.\n\nIf discovery still fails, read `tynd://errors/DEBUG_SOCKET_MISSING`.",
        env!("CARGO_PKG_VERSION"),
        discovery,
        attached,
    )
}

const DEBUG_IPC_ERROR: &str = r"You will diagnose a failing IPC call in the attached Tynd app. Follow these steps in order.

1. Call `ipc.failed` and select the most recent failed frame.
2. Read its `error.code` and open `tynd://errors/<code>`.
3. Call `app.logs.tail` with `n=200` for the host-side stack near that timestamp.
4. Compare the failing payload with the backend validator; use `plugin.command_catalog` for plugin calls.
5. Propose one concrete fix: change the call site, validator, or capability grant.
6. Run the flow again and verify with `ipc.failed`.

Quote the real payload, error code, and exact fix. Do not speculate beyond tool evidence.";

const FIX_WEBVIEW_ERRORS: &str = r"You will triage frontend errors in the attached Tynd app webview.

1. Call `webview.console_logs` and `webview.network_requests` with no `since`.
2. Group results into JavaScript exceptions, blocked navigation/CSP, and 4xx/5xx responses.
3. Propose the smallest fix for each group. For blocked navigation, read `tynd://errors/WEBVIEW_NAVIGATION_BLOCKED`.
4. Ask the user to refresh and rerun the checks after each fix.

Quote real log lines, URLs, and status codes. Do not invent symptoms.";

const FIX_PERMISSION_DENIED: &str = r"A backend RPC failed with `PERMISSION_DENIED`. Resolve it by granting the exact capability.

1. Call `ipc.failed` and find the latest `PERMISSION_DENIED` frame; note its `fn`.
2. Call `capability.suggest_grant` with that API.
3. Add the returned `capability` object to `app.start({ capabilities: [...] })`; scope `windows` to the calling window.
4. Restart `tynd dev` and rerun the failing flow.
5. Call `capability.check` for the same API; it must report granted.

Do not skip verification.";

const VISUAL_REGRESSION: &str = r"Run a visual regression pass on the attached Tynd app.

1. Call `window.list`.
2. Capture each window with `screenshot.thumbnail`.
3. Compare layout, labels, error banners, modal state, and focus against the expected description or previous run.
4. Use `screenshot.window` only where a thumbnail shows a possible delta.
5. Report: window label | observation | severity (info / warn / break).

Only flag real regressions.";

const EXPLAIN_STACK: &str = r"Explain the most recent host stack trace in the attached Tynd app.

1. Call `app.logs.tail` with `n=300`; find the latest error with a stack.
2. Identify the top user-code frame, skipping host and node_modules frames.
3. Read the relevant `tynd://docs/<topic>` resource when a Tynd subsystem is involved.
4. Explain what the runtime attempted, why it failed, and the concrete fix.
5. Cite the exact log line.

If there is no recent error, say so and stop.";
