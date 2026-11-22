//! Bridges the debug protocol to the existing webview eval channel.
//!
//! The host already owns a `wry::WebView` per window; we expose a
//! callback registry that the debug methods invoke to ship JS into the
//! main webview without dragging a `WebView` reference through the
//! protocol layer (wry handles aren't `Send` on every platform).

use std::sync::OnceLock;

use serde_json::Value;

/// Function signature used to evaluate JS in a named webview from
/// arbitrary threads. Returns the JS-serialized result captured via
/// `wry::WebView::evaluate_script_with_callback` or an error string.
///
/// `label` is the window label (`"main"` for the primary window). The
/// installer is responsible for routing the eval onto the main thread.
pub(crate) type EvalFn = Box<dyn Fn(&str, &str) -> Result<Value, String> + Send + Sync + 'static>;

static EVAL_HOOK: OnceLock<EvalFn> = OnceLock::new();

/// Install the eval hook from the host bootstrap. Called once at
/// startup when the debug protocol is enabled.
pub(crate) fn set_eval_hook(f: EvalFn) {
    let _ = EVAL_HOOK.set(f);
}

/// Eval `script` in the webview labelled `label`. Returns
/// `Err("eval hook not installed")` when called before the host
/// bootstrap has wired the channel — which is the case in unit tests
/// that don't start the event loop.
pub(crate) fn eval(label: &str, script: &str) -> Result<Value, String> {
    let hook = EVAL_HOOK.get().ok_or("eval hook not installed")?;
    hook(label, script)
}

/// JS helper injected into every webview when the debug protocol is on.
/// Available via `window.__tynd_debug_*__` globals.
pub(crate) const DEBUG_HELPER_JS: &str = include_str!("debug_helper.js");
