use serde_json::{json, Value};

// ── JS scripts baked into the binary ─────────────────────────────────────────

/// Fires `{__bv_page_ready:true}` on the first real DOMContentLoaded.
/// The host uses this to show the window only when content is actually rendered,
/// eliminating the white-flash on startup.
pub const JS_PAGE_READY: &str = r#"
(function () {
  window.addEventListener("DOMContentLoaded", function () {
    var url = document.URL;
    // Skip the initial blank placeholder loaded by with_html()
    if (url && url !== "about:blank" && !url.startsWith("data:")) {
      window.ipc.postMessage(JSON.stringify({ __bv_page_ready: true }));
    }
  });
})();
"#;

/// Wry IPC shim: translates window.ipc.postMessage() → __bunview__ / __bv_emit__ API.
/// Injected first (before CLIENT_SCRIPT) via with_initialization_script.
pub const JS_SHIM: &str = r#"
(function () {
  "use strict";
  var __bv_pending = new Map();
  var __bv_seq = 0;

  // Called by Rust to resolve a pending __bunview__ promise.
  window.__bv_resolve = function (id, status, result) {
    var p = __bv_pending.get(String(id));
    if (p) {
      __bv_pending.delete(String(id));
      if (status === 0) { p.resolve(result); } else { p.reject(new Error(String(result))); }
    }
  };

  // RPC: JS → Bun → JS.  Returns a Promise.
  window.__bunview__ = function (jsonStr) {
    return new Promise(function (resolve, reject) {
      var id = String(++__bv_seq);
      __bv_pending.set(id, { resolve: resolve, reject: reject });
      window.ipc.postMessage(JSON.stringify({ __bv_id: id, __bv_data: jsonStr }));
    });
  };

  // Event channel: frontend → backend (fire-and-forget).
  window.__bv_emit__ = function (jsonStr) {
    window.ipc.postMessage(JSON.stringify({ __bv_emit: true, __bv_data: jsonStr }));
    return Promise.resolve(null);
  };
})();
"#;

/// Bunview public client API — creates window.__bunview_api__.
/// Must run AFTER JS_SHIM.
pub const CLIENT_SCRIPT: &str = r#"
(function () {
  var listeners = new Map();

  function _dispatch(name, payload) {
    var handlers = listeners.get(name);
    if (handlers) { for (var cb of handlers) { try { cb(payload); } catch (e) {} } }
  }

  function invoke(command, payload) {
    if (typeof window.__bunview__ !== "function") {
      return Promise.reject(new Error("[bunview] Native IPC bridge not available."));
    }
    return window.__bunview__(JSON.stringify({ command: command, payload: payload }))
      .then(function (raw) {
        if (raw === null || raw === undefined) return raw;
        if (typeof raw === "string") { try { return JSON.parse(raw); } catch { return raw; } }
        return raw;
      });
  }

  function emit(event, payload) {
    if (typeof window.__bv_emit__ === "function") {
      window.__bv_emit__(JSON.stringify({ name: event, payload: payload }));
    }
  }

  function listen(event, handler) {
    if (!listeners.has(event)) listeners.set(event, new Set());
    listeners.get(event).add(handler);
    return function () { var s = listeners.get(event); if (s) s.delete(handler); };
  }

  function once(event, handler) {
    var off = listen(event, function (payload) { off(); handler(payload); });
    return off;
  }

  window.__bunview_api__ = {
    invoke:   invoke,
    listen:   listen,
    once:     once,
    emit:     emit,
    _dispatch: _dispatch,
    isConnected:      function () { return true; },
    onConnectionChange: function (cb) { cb(true); return function () {}; },
  };
})();
"#;

// ── stdout helpers ────────────────────────────────────────────────────────────

pub fn emit(v: Value) {
    println!("{v}");
}

pub fn emit_ready() {
    emit(json!({"type": "ready"}));
}

pub fn emit_close() {
    emit(json!({"type": "close"}));
}

pub fn emit_response(id: &str, result: Value) {
    emit(json!({"type": "response", "id": id, "result": result}));
}

pub fn emit_error_response(id: &str, error: &str) {
    emit(json!({"type": "response", "id": id, "error": error}));
}

// ── IPC handler (called from wry main-thread IPC callback) ────────────────────
//
// body is what JS passed to window.ipc.postMessage().
// Two message shapes:
//   Invoke:  {"__bv_id":"1","__bv_data":"{\"command\":\"greet\",\"payload\":\"World\"}"}
//   Event:   {"__bv_emit":true,"__bv_data":"{\"name\":\"click\",\"payload\":{}}"}
/// Returns `true` if `body` is the internal page-ready signal (`{__bv_page_ready:true}`).
/// The caller should then send a `ShowWindow` user-event via the event-loop proxy.
pub fn is_page_ready(body: &str) -> bool {
    serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| v.get("__bv_page_ready").and_then(|f| f.as_bool()))
        .unwrap_or(false)
}

pub fn handle_ipc(body: &str) {
    let Ok(outer) = serde_json::from_str::<Value>(body) else { return };

    // Dialog overlay result has no __bv_data — check it first to avoid the early return below.
    if outer.get("__bv_dialog_result").is_some() {
        let id    = outer.get("__bv_dialog_id").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let value = outer.get("value").cloned().unwrap_or(Value::Null);
        emit(json!({"type": "response", "id": id, "result": value}));
        return;
    }

    let Some(data_str) = outer.get("__bv_data").and_then(|v| v.as_str()) else { return };
    let Ok(inner) = serde_json::from_str::<Value>(data_str) else { return };

    if outer.get("__bv_emit").and_then(|v| v.as_bool()).unwrap_or(false) {
        // Frontend event → forward to Bun
        let name    = inner.get("name").and_then(|v| v.as_str()).unwrap_or("");
        let payload = inner.get("payload").cloned().unwrap_or(Value::Null);
        emit(json!({"type": "frontendEvent", "name": name, "payload": payload}));
    } else if let Some(id) = outer.get("__bv_id").and_then(|v| v.as_str()) {
        // RPC invocation → forward to Bun
        let command = inner.get("command").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let payload = inner.get("payload").cloned().unwrap_or(Value::Null);
        emit(json!({"type": "invoke", "id": id, "command": command, "payload": payload}));
    }
}

// ── WebView eval helpers ──────────────────────────────────────────────────────

/// Build the eval string that resolves a pending JS promise.
pub fn eval_resolve(id: &str, status: u8, result: &Value) -> String {
    let result_js = serde_json::to_string(result).unwrap_or_else(|_| "null".into());
    format!("window.__bv_resolve&&window.__bv_resolve({},{},{result_js})",
        serde_json::to_string(id).unwrap(),
        status)
}

/// Build the eval string that dispatches a named event to the frontend.
pub fn eval_dispatch(name: &str, payload: &Value) -> String {
    let name_js    = serde_json::to_string(name).unwrap_or_else(|_| "\"\"".into());
    let payload_js = serde_json::to_string(payload).unwrap_or_else(|_| "null".into());
    format!("window.__bunview_api__&&window.__bunview_api__._dispatch({name_js},{payload_js})")
}
