//! `webview.*` debug methods — eval, snapshot, click, type, find_element,
//! scroll, console_logs, network_requests.
//!
//! Most operations route through the JS helper injected at startup
//! (`debug_helper.js`). The eval channel is provided by `webview_glue`.

use serde_json::{json, Value};
use tynd_debug_protocol::protocol::RpcError;

use super::CallOutcome;
use crate::webview::shim::js_val;

use crate::debug::webview_glue;

fn label_from(params: &Value) -> &str {
    params
        .get("label")
        .and_then(Value::as_str)
        .unwrap_or("main")
}

fn run_helper(label: &str, expr: &str) -> Result<Value, String> {
    // Wrap in `JSON.stringify(...)` so the result we get back is always
    // a plain string — wry's evaluate_script_with_callback feeds back
    // JSON-encoded values. WebView2 discards uncaught JS exceptions and
    // reports `null`, so catch inside the page and preserve the cause.
    let wrapped = helper_script(expr);
    let raw = webview_glue::eval(label, &wrapped)?;
    let value = if let Value::String(s) = &raw {
        serde_json::from_str::<Value>(s).map_err(|e| format!("decode result: {e}"))
    } else {
        Ok(raw)
    }?;
    if let Some(error) = value.get("__tynd_debug_error__").and_then(Value::as_str) {
        return Err(format!("webview helper: {error}"));
    }
    Ok(value)
}

fn helper_script(expr: &str) -> String {
    let bootstrap = crate::debug::DEBUG_HELPER_JS;
    format!(
        "(() => {{ try {{ \
         if (typeof window.__tynd_debug_snapshot__ !== 'function') {{ \
         delete window.__tynd_debug_helper_loaded__; {bootstrap} }} \
         return JSON.stringify({expr}); }} catch (error) {{ \
         return JSON.stringify({{ __tynd_debug_error__: String(error) }}); }} }})()"
    )
}

pub(crate) fn eval(params: Value) -> CallOutcome {
    let Some(script) = params.get("script").and_then(Value::as_str) else {
        return CallOutcome::Error(RpcError::invalid_params("missing 'script'"));
    };
    let label = label_from(&params);
    match webview_glue::eval(label, script) {
        Ok(v) => CallOutcome::Result(v),
        Err(e) => CallOutcome::Error(RpcError::internal_error(e)),
    }
}

pub(crate) fn snapshot(params: Value) -> CallOutcome {
    let mode = params.get("mode").and_then(Value::as_str).unwrap_or("map");
    let expr = format!("__tynd_debug_snapshot__({})", js_val(&json!(mode)));
    match run_helper(label_from(&params), &expr) {
        Ok(v) => CallOutcome::Result(v),
        Err(e) => CallOutcome::Error(RpcError::internal_error(e)),
    }
}

pub(crate) fn click(params: Value) -> CallOutcome {
    let target = params.get("target").cloned().unwrap_or(Value::Null);
    // SECURITY: route every user-controlled value through `js_val` so
    // embedded U+2028/U+2029 in a string can't terminate the JS line and
    // smuggle arbitrary code into the helper invocation. Same hole
    // `js_escape_line_terminators` closes for `eval_resolve` / `eval_yield`
    // in `ipc.rs`.
    let expr = format!("__tynd_debug_click__({})", js_val(&target));
    match run_helper(label_from(&params), &expr) {
        Ok(v) => CallOutcome::Result(v),
        Err(e) => CallOutcome::Error(RpcError::internal_error(e)),
    }
}

pub(crate) fn type_text(params: Value) -> CallOutcome {
    let Some(text) = params
        .get("text")
        .and_then(Value::as_str)
        .map(str::to_string)
    else {
        return CallOutcome::Error(RpcError::invalid_params("missing 'text'"));
    };
    let target = params.get("target").cloned().unwrap_or(Value::Null);
    let text_json = json!(text);
    let expr = format!(
        "__tynd_debug_type__({}, {})",
        js_val(&text_json),
        js_val(&target),
    );
    match run_helper(label_from(&params), &expr) {
        Ok(v) => CallOutcome::Result(v),
        Err(e) => CallOutcome::Error(RpcError::internal_error(e)),
    }
}

pub(crate) fn fill(params: Value) -> CallOutcome {
    let Some(fields) = params.get("fields").and_then(Value::as_array) else {
        return CallOutcome::Error(RpcError::invalid_params("missing 'fields'"));
    };
    let expr = format!("__tynd_debug_fill__({})", js_val(&json!(fields)));
    match run_helper(label_from(&params), &expr) {
        Ok(v) => CallOutcome::Result(v),
        Err(e) => CallOutcome::Error(RpcError::internal_error(e)),
    }
}

pub(crate) fn press_key(params: Value) -> CallOutcome {
    let Some(key) = params.get("key").and_then(Value::as_str) else {
        return CallOutcome::Error(RpcError::invalid_params("missing 'key'"));
    };
    let expr = format!("__tynd_debug_press_key__({})", js_val(&json!(key)));
    match run_helper(label_from(&params), &expr) {
        Ok(v) => CallOutcome::Result(v),
        Err(e) => CallOutcome::Error(RpcError::internal_error(e)),
    }
}

pub(crate) fn find_element(params: Value) -> CallOutcome {
    let Some(selector) = params
        .get("selector")
        .and_then(Value::as_str)
        .map(str::to_string)
    else {
        return CallOutcome::Error(RpcError::invalid_params("missing 'selector'"));
    };
    let expr = format!("__tynd_debug_find__({})", js_val(&json!(selector)));
    match run_helper(label_from(&params), &expr) {
        Ok(v) => CallOutcome::Result(v),
        Err(e) => CallOutcome::Error(RpcError::internal_error(e)),
    }
}

pub(crate) fn scroll(params: Value) -> CallOutcome {
    let target = params.get("target").cloned().unwrap_or(Value::Null);
    let opts = params.get("opts").cloned().unwrap_or(Value::Null);
    let expr = format!(
        "__tynd_debug_scroll__({}, {})",
        js_val(&target),
        js_val(&opts),
    );
    match run_helper(label_from(&params), &expr) {
        Ok(v) => CallOutcome::Result(v),
        Err(e) => CallOutcome::Error(RpcError::internal_error(e)),
    }
}

pub(crate) fn hover(params: Value) -> CallOutcome {
    let target = params.get("target").cloned().unwrap_or(Value::Null);
    let expr = format!("__tynd_debug_hover__({})", js_val(&target));
    match run_helper(label_from(&params), &expr) {
        Ok(v) => CallOutcome::Result(v),
        Err(e) => CallOutcome::Error(RpcError::internal_error(e)),
    }
}

pub(crate) fn drag(params: Value) -> CallOutcome {
    let start = params.get("start").cloned().unwrap_or(Value::Null);
    let end = params.get("end").cloned().unwrap_or(Value::Null);
    let expr = format!("__tynd_debug_drag__({}, {})", js_val(&start), js_val(&end));
    match run_helper(label_from(&params), &expr) {
        Ok(v) => CallOutcome::Result(v),
        Err(e) => CallOutcome::Error(RpcError::internal_error(e)),
    }
}

fn filter_since(value: Value, since: Option<u64>) -> Value {
    let Some(since) = since else { return value };
    let Value::Array(entries) = value else {
        return Value::Array(Vec::new());
    };
    Value::Array(
        entries
            .into_iter()
            .filter(|entry| {
                entry
                    .get("ts")
                    .and_then(Value::as_u64)
                    .is_some_and(|ts| ts >= since)
            })
            .collect(),
    )
}

pub(crate) fn console_logs(params: Value) -> CallOutcome {
    let clear = params
        .get("clear")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let expr = format!("__tynd_debug_console__({})", js_val(&json!(clear)));
    match run_helper(label_from(&params), &expr) {
        Ok(v) => CallOutcome::Result(filter_since(v, params.get("since").and_then(Value::as_u64))),
        Err(e) => CallOutcome::Error(RpcError::internal_error(e)),
    }
}

pub(crate) fn network_requests(params: Value) -> CallOutcome {
    let clear = params
        .get("clear")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let expr = format!("__tynd_debug_network__({})", js_val(&json!(clear)));
    match run_helper(label_from(&params), &expr) {
        Ok(v) => CallOutcome::Result(filter_since(v, params.get("since").and_then(Value::as_u64))),
        Err(e) => CallOutcome::Error(RpcError::internal_error(e)),
    }
}
