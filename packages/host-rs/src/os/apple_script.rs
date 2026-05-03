//! AppleScript runner — exposes `os.appleScript.run(script)` to the
//! frontend behind the `api-apple-script` feature.
//!
//! Useful for OS-level integrations the webview can't perform
//! natively:
//!  - Post-update relaunch via `tell application "System Events"
//!    to (do shell script ...)` instead of the brittle
//!    `LaunchServices` re-spawn dance.
//!  - Calendar / Reminders / Mail / Finder bookkeeping.
//!  - Vendor automation that expects an OSA host (Adobe, Logic, etc).
//!
//! Implementation: `osakit::Script::new_from_source(source).execute()`
//! returns a JSON `Value` already — we forward it as-is so callers
//! can `JSON.parse`-like access from the JS side.
//!
//! macOS only — the entire module is `#[cfg(target_os = "macos")]`
//! so dispatch on Linux/Windows short-circuits to `api_disabled(...)`
//! upstream.

#![cfg(all(target_os = "macos", feature = "api-apple-script"))]

use serde::Deserialize;
use serde_json::{json, Value};

use crate::ipc_error::IpcError;

/// IPC entry point. Dispatched from `os::mod.rs` table.
pub fn dispatch(method: &str, args: &Value) -> Result<Value, String> {
    match method {
        "run" => run(args),
        _ => Err(IpcError::unknown_method("appleScript", method).into_string()),
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RunArgs {
    /// AppleScript source, single string. Multi-line is supported
    /// (osakit forwards verbatim to the OSA component).
    script: String,
    /// Optional language override — `"AppleScript"` (default) or
    /// `"JavaScript"` for JXA. Anything else returns a typed error
    /// instead of falling through to the OSA component which would
    /// produce a less actionable failure.
    #[serde(default)]
    language: Option<String>,
}

fn run(args: &Value) -> Result<Value, String> {
    let a: RunArgs = serde_json::from_value(args.clone()).map_err(|e| {
        IpcError::new("appleScript", "run", format!("invalid arguments: {e}")).into_string()
    })?;

    let language = a.language.as_deref().unwrap_or("AppleScript");
    let lang = match language {
        "AppleScript" => osakit::Language::AppleScript,
        "JavaScript" | "JXA" => osakit::Language::JavaScript,
        other => {
            return Err(IpcError::new(
                "appleScript",
                "run",
                format!("unsupported language '{other}' (expected 'AppleScript' or 'JavaScript')"),
            )
            .into_string());
        },
    };

    let mut script = osakit::Script::new_from_source(lang, &a.script);
    script
        .compile()
        .map_err(|e| IpcError::new("appleScript", "run", format!("compile: {e}")).into_string())?;
    let value = script
        .execute()
        .map_err(|e| IpcError::new("appleScript", "run", format!("execute: {e}")).into_string())?;

    Ok(json!({ "value": value }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Smoke test that doesn't actually invoke OSA — verifies the arg
    /// parser rejects an unknown language with the expected error
    /// shape. Real script execution is gated by the OS environment
    /// (CI macOS hosts), so we cover that path in integration tests.
    #[test]
    fn rejects_unknown_language() {
        let err = run(&json!({ "script": "true", "language": "Python" })).unwrap_err();
        assert!(err.contains("unsupported language"));
    }

    #[test]
    fn rejects_missing_script() {
        let err = run(&json!({ "language": "AppleScript" })).unwrap_err();
        assert!(err.contains("invalid arguments"));
    }
}
