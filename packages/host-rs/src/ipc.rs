use serde_json::Value;

// ── JS scripts injected into every WebView page ───────────────────────────────

/// Injected only in debug mode (`--debug` / `vorn dev`).
/// Sets `window.__vorn_dev__ = true` so framework JS can prefix its own
/// console messages with `[vorn]` without leaking the name in production.
pub const JS_DEV_FLAG: &str = "window.__vorn_dev__ = true;";

/// Fires the page-ready signal on DOMContentLoaded (skips the initial blank page).
/// Rust intercepts this to show the window only after real content is rendered,
/// eliminating the white-flash on startup.
pub const JS_PAGE_READY: &str = r#"
(function () {
  document.addEventListener("DOMContentLoaded", function () {
    var url = document.URL;
    if (url && url !== "about:blank" && !url.startsWith("data:")) {
      window.ipc.postMessage(JSON.stringify({ __vorn_page_ready: true }));
    }
  });
})();
"#;

/// Core IPC shim — creates window.__vorn__ with call/on/off/os_call.
/// Injected before any page script via with_initialization_script.
pub const JS_SHIM: &str = r#"
(function () {
  "use strict";

  var _seq = 0;
  var _pending = {};
  var _os_pending = {};
  var _listeners = {};
  var _os_listeners = {};
  // Internal logger: prefixes with [vorn] in dev mode only (window.__vorn_dev__ injected by vorn dev)
  var _log = window.__vorn_dev__
    ? function(msg) { console.error("[vorn] " + msg); }
    : function() {};

  // Called by Rust: resolve/reject a pending backend call promise
  window.__vorn_resolve__ = function (id, ok, value) {
    var p = _pending[id];
    if (!p) return;
    delete _pending[id];
    if (ok) { p.resolve(value); } else { p.reject(new Error(String(value))); }
  };

  // Called by Rust: resolve/reject a pending OS API call promise
  window.__vorn_os_result__ = function (id, ok, value) {
    var p = _os_pending[id];
    if (!p) return;
    delete _os_pending[id];
    if (ok) { p.resolve(value); } else { p.reject(new Error(String(value))); }
  };

  // Called by Rust: deliver a native OS event (menu click, tray click, …)
  window.__vorn_os_event__ = function (name, data) {
    var handlers = _os_listeners[name];
    if (!handlers) return;
    for (var i = 0; i < handlers.length; i++) {
      try { handlers[i](data); } catch (e) {
        _log("os event handler error: " + e);
      }
    }
  };

  // Called by Rust: dispatch a named event to frontend subscribers
  window.__vorn_dispatch__ = function (name, payload) {
    var handlers = _listeners[name];
    if (!handlers) return;
    for (var i = 0; i < handlers.length; i++) {
      try { handlers[i](payload); } catch (e) {
        _log("event handler error: " + e);
      }
    }
  };

  window.__vorn__ = {
    // Call a backend function — returns a Promise
    call: function (fn, args) {
      return new Promise(function (resolve, reject) {
        var id = String(++_seq);
        _pending[id] = { resolve: resolve, reject: reject };
        window.ipc.postMessage(
          JSON.stringify({ type: "call", id: id, fn: fn, args: args })
        );
      });
    },

    // Subscribe to a native OS event (menu:action, tray:click, …)
    // Returns an unsubscribe function.
    os_on: function (name, handler) {
      if (!_os_listeners[name]) _os_listeners[name] = [];
      _os_listeners[name].push(handler);
      return function () {
        var list = _os_listeners[name];
        if (!list) return;
        var idx = list.indexOf(handler);
        if (idx >= 0) list.splice(idx, 1);
      };
    },

    // Call a native OS API — returns a Promise
    os_call: function (api, method, args) {
      return new Promise(function (resolve, reject) {
        var id = "os_" + String(++_seq);
        _os_pending[id] = { resolve: resolve, reject: reject };
        window.ipc.postMessage(
          JSON.stringify({ type: "os_call", id: id, api: api, method: method, args: args })
        );
      });
    },

    // Subscribe to a backend event — returns an unsubscribe function
    on: function (name, handler) {
      if (!_listeners[name]) _listeners[name] = [];
      _listeners[name].push(handler);
      return function () {
        var list = _listeners[name];
        if (!list) return;
        var idx = list.indexOf(handler);
        if (idx >= 0) list.splice(idx, 1);
      };
    },

    // Unsubscribe a specific handler
    off: function (name, handler) {
      var list = _listeners[name];
      if (!list) return;
      var idx = list.indexOf(handler);
      if (idx >= 0) list.splice(idx, 1);
    },
  };
})();
"#;

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Build the JS eval string that resolves/rejects a pending frontend call.
#[inline]
pub fn eval_resolve(id: &str, ok: bool, value: &Value) -> String {
    let ok_js  = if ok { "true" } else { "false" };
    let val_js = serde_json::to_string(value).unwrap_or_else(|_| "null".into());
    let id_js  = serde_json::to_string(id).unwrap_or_else(|_| "\"\"".into());
    format!("window.__vorn_resolve__&&window.__vorn_resolve__({id_js},{ok_js},{val_js})")
}

/// Build the JS eval string that dispatches an event to frontend subscribers.
#[inline]
pub fn eval_dispatch(name: &str, payload: &Value) -> String {
    let name_js    = serde_json::to_string(name).unwrap_or_else(|_| "\"\"".into());
    let payload_js = serde_json::to_string(payload).unwrap_or_else(|_| "null".into());
    format!("window.__vorn_dispatch__&&window.__vorn_dispatch__({name_js},{payload_js})")
}

/// Build the JS eval string that resolves/rejects a pending OS API call.
#[inline]
pub fn eval_os_result(id: &str, ok: bool, value: &Value) -> String {
    let ok_js  = if ok { "true" } else { "false" };
    let val_js = serde_json::to_string(value).unwrap_or_else(|_| "null".into());
    let id_js  = serde_json::to_string(id).unwrap_or_else(|_| "\"\"".into());
    format!("window.__vorn_os_result__&&window.__vorn_os_result__({id_js},{ok_js},{val_js})")
}

/// Build the JS eval string that fires a native OS event to subscribed handlers.
#[inline]
pub fn eval_os_event(name: &str, data: &Value) -> String {
    let name_js = serde_json::to_string(name).unwrap_or_else(|_| "\"\"".into());
    let data_js = serde_json::to_string(data).unwrap_or_else(|_| "null".into());
    format!("window.__vorn_os_event__&&window.__vorn_os_event__({name_js},{data_js})")
}
