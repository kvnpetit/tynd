//! AppleScript runner — `os.appleScript.run(script)` behind `api-apple-script`.
//!
//! Useful for OS integrations the webview can't perform: post-update relaunch,
//! Calendar/Reminders/Finder bookkeeping, OSA-hosted vendor automation.
//!
//! macOS only — the whole module is gated; non-macOS dispatches return
//! `api_disabled(...)` upstream.
//!
//! `run` is ACL-gated: arbitrary OSA/JXA execution is the most powerful
//! capability surfaced by the host, so it must be explicitly granted.

#![cfg(all(target_os = "macos", feature = "api-apple-script"))]

use serde_json::{json, Value};

use crate::api::HostApi;
use crate::ipc::error::IpcError;

// Public methods are macro inputs; keeping the module crate-private prevents ACL bypass.
#[allow(unreachable_pub)]
pub(crate) mod inner {
    use serde_json::Value;

    use super::run_impl;
    use crate::tynd;

    /// Stateless dispatch marker.
    #[derive(Debug, Default)]
    pub(crate) struct AppleScriptApi;

    #[tynd::handler(api = "appleScript")]
    impl AppleScriptApi {
        /// Compile + execute an AppleScript (or JXA, when `language` is
        /// `"JavaScript"` / `"JXA"`) and return the OSA component's
        /// JSON-shaped output under a `{ value: ... }` envelope.
        pub fn run(script: String, language: Option<String>) -> Result<Value, String> {
            run_impl(&script, language.as_deref())
        }
    }
}

/// Row of the host API table (`api::HOST_APIS`).
pub(crate) const API: HostApi = HostApi::new("appleScript", |method, args| dispatch(method, &args));

/// Public dispatch wrapper — interposes the ACL gate on `run` before
/// delegating to the macro-generated table inside [`inner`].
pub(crate) fn dispatch(method: &str, args: &Value) -> Result<Value, String> {
    if method == "run"
        && !crate::security::acl::authority().is_allowed("appleScript:run", None, None, None)
    {
        return Err(IpcError::new("appleScript", "run", "permission denied by ACL").into_string());
    }
    inner::dispatch(method, args)
}

fn run_impl(script_src: &str, language: Option<&str>) -> Result<Value, String> {
    let lang = match language.unwrap_or("AppleScript") {
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

    let mut script = osakit::Script::new_from_source(lang, script_src);
    script
        .compile()
        .map_err(|e| IpcError::new("appleScript", "run", format!("compile: {e}")).into_string())?;
    let value = script
        .execute()
        .map_err(|e| IpcError::new("appleScript", "run", format!("execute: {e}")).into_string())?;

    Ok(json!({ "value": value }))
}
