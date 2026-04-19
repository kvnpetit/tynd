use serde_json::Value;

/// Injected only in debug mode (`--debug` / `tynd dev`).
/// Sets `window.__tynd_dev__ = true` so framework JS can prefix its own
/// console messages with `[tynd]` without leaking the name in production.
pub const JS_DEV_FLAG: &str = "window.__tynd_dev__ = true;";

/// Fires the page-ready signal on DOMContentLoaded (skips the initial blank page).
/// Rust intercepts this to show the window only after real content is rendered,
/// eliminating the white-flash on startup.
pub const JS_PAGE_READY: &str = r#"
(function () {
  document.addEventListener("DOMContentLoaded", function () {
    var url = document.URL;
    if (url && url !== "about:blank" && !url.startsWith("data:")) {
      window.ipc.postMessage(JSON.stringify({ __tynd_page_ready: true }));
    }
  });
})();
"#;

/// Core IPC shim — creates window.__tynd__ with call/on/off/os_call.
/// Injected before any page script via with_initialization_script.
pub const JS_SHIM: &str = r#"
(function () {
  "use strict";

  var _seq = 0;
  var _pending = {};
  var _os_pending = {};
  var _listeners = {};
  var _os_listeners = {};
  // Internal logger: prefixes with [tynd] in dev mode only (window.__tynd_dev__ injected by tynd dev)
  var _log = window.__tynd_dev__
    ? function(msg) { console.error("[tynd] " + msg); }
    : function() {};

  // Called by Rust: resolve/reject a pending backend call promise + close its stream.
  window.__tynd_resolve__ = function (id, ok, value) {
    var p = _pending[id];
    if (!p) return;
    delete _pending[id];
    if (ok) {
      p.finalValue = value;
      p.done = true;
      p.resolve(value);
      // Drain any async iterator waiters with { done: true }
      while (p.waiters.length) {
        var w = p.waiters.shift();
        w.resolve({ value: undefined, done: true });
      }
    } else {
      p.done = true;
      p.error = new Error(String(value));
      p.reject(p.error);
      while (p.waiters.length) {
        var w2 = p.waiters.shift();
        w2.reject(p.error);
      }
    }
  };

  // Chunks are acked back to the backend every ACK_CHUNK consumed — lets
  // the backend keep ~STREAM_CREDIT chunks in-flight before it has to stall.
  // Big enough to hide round-trip latency, small enough to bound memory.
  var ACK_CHUNK = 32;
  function _ackConsume(id, p) {
    p.consumed = (p.consumed | 0) + 1;
    if (p.consumed >= ACK_CHUNK) {
      try {
        window.ipc.postMessage(JSON.stringify({ type: "ack", id: id, n: p.consumed }));
      } catch (_) {}
      p.consumed = 0;
    }
  }

  // Called by Rust: deliver one chunk from a streaming backend call.
  window.__tynd_yield__ = function (id, value) {
    var p = _pending[id];
    if (!p || p.done) return;
    if (p.waiters.length) {
      p.waiters.shift().resolve({ value: value, done: false });
      _ackConsume(id, p);
    } else {
      p.yields.push(value);
    }
  };

  // Called by Rust: deliver a batch of yields in one call to minimize
  // `evaluate_script` overhead on bursty streams. `pairs` is an array of
  // [id, value] tuples in FIFO order.
  window.__tynd_yield_batch__ = function (pairs) {
    for (var i = 0; i < pairs.length; i++) {
      window.__tynd_yield__(pairs[i][0], pairs[i][1]);
    }
  };

  // Called by Rust: resolve/reject a pending OS API call promise
  window.__tynd_os_result__ = function (id, ok, value) {
    var p = _os_pending[id];
    if (!p) return;
    delete _os_pending[id];
    if (ok) { p.resolve(value); } else { p.reject(new Error(String(value))); }
  };

  // Called by Rust: deliver a native OS event (menu click, tray click, …)
  window.__tynd_os_event__ = function (name, data) {
    var handlers = _os_listeners[name];
    if (!handlers) return;
    for (var i = 0; i < handlers.length; i++) {
      try { handlers[i](data); } catch (e) {
        _log("os event handler error: " + e);
      }
    }
  };

  // Called by Rust: dispatch a named event to frontend subscribers
  window.__tynd_dispatch__ = function (name, payload) {
    var handlers = _listeners[name];
    if (!handlers) return;
    for (var i = 0; i < handlers.length; i++) {
      try { handlers[i](payload); } catch (e) {
        _log("event handler error: " + e);
      }
    }
  };

  window.__tynd__ = {
    // Call a backend function — returns an awaitable+iterable handle.
    // `await call(...)` -> final return value (regular async handlers, or
    // the generator's `return` value for async-generators).
    // `for await (x of call(...))` -> yielded chunks.
    call: function (fn, args) {
      var id = String(++_seq);
      var p;
      var finalPromise = new Promise(function (res, rej) {
        p = {
          resolve: res, reject: rej,
          yields: [], waiters: [],
          done: false, error: null, finalValue: undefined, cancelled: false,
        };
      });
      _pending[id] = p;

      var iterator = {
        next: function () {
          return new Promise(function (resolve, reject) {
            if (p.yields.length) {
              resolve({ value: p.yields.shift(), done: false });
              _ackConsume(id, p);
              return;
            }
            if (p.done) {
              if (p.error) reject(p.error);
              else resolve({ value: undefined, done: true });
              return;
            }
            p.waiters.push({ resolve: resolve, reject: reject });
          });
        },
        "return": function (value) {
          if (!p.done && !p.cancelled) {
            p.cancelled = true;
            try {
              window.ipc.postMessage(JSON.stringify({ type: "cancel", id: id }));
            } catch (_) {}
          }
          return Promise.resolve({ value: value, done: true });
        },
        "throw": function (err) { return Promise.reject(err); },
      };
      iterator[Symbol.asyncIterator] = function () { return iterator; };

      var handle = {
        then: function (onF, onR) { return finalPromise.then(onF, onR); },
        "catch": function (onR) { return finalPromise["catch"](onR); },
        "finally": function (onF) { return finalPromise["finally"](onF); },
        cancel: function () { return iterator["return"](); },
      };
      handle[Symbol.asyncIterator] = function () { return iterator; };

      window.ipc.postMessage(
        JSON.stringify({ type: "call", id: id, fn: fn, args: args })
      );
      return handle;
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

/// Build the JS eval string that resolves/rejects a pending frontend call.
#[inline]
pub fn eval_resolve(id: &str, ok: bool, value: &Value) -> String {
    let ok_js = if ok { "true" } else { "false" };
    let val_js = serde_json::to_string(value).unwrap_or_else(|_| "null".into());
    let id_js = serde_json::to_string(id).unwrap_or_else(|_| "\"\"".into());
    format!("window.__tynd_resolve__&&window.__tynd_resolve__({id_js},{ok_js},{val_js})")
}

/// Build the JS eval string that delivers one chunk from a streaming backend call.
#[inline]
pub fn eval_yield(id: &str, value: &Value) -> String {
    let val_js = serde_json::to_string(value).unwrap_or_else(|_| "null".into());
    let id_js = serde_json::to_string(id).unwrap_or_else(|_| "\"\"".into());
    format!("window.__tynd_yield__&&window.__tynd_yield__({id_js},{val_js})")
}

/// Build the JS eval string that delivers a batched flush of streaming chunks.
/// `pairs` is `[(id, value)]` and renders to `__tynd_yield_batch__([[id,val],…])`.
/// Single `evaluate_script` instead of one per yield — cuts main-thread cost
/// ~30x on bursty streams (e.g. 10k tokens/s).
#[inline]
pub fn eval_yield_batch(pairs: &[(String, Value)]) -> String {
    let mut out = String::with_capacity(pairs.len() * 48);
    out.push_str("window.__tynd_yield_batch__&&window.__tynd_yield_batch__([");
    for (i, (id, value)) in pairs.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push('[');
        out.push_str(&serde_json::to_string(id).unwrap_or_else(|_| "\"\"".into()));
        out.push(',');
        out.push_str(&serde_json::to_string(value).unwrap_or_else(|_| "null".into()));
        out.push(']');
    }
    out.push_str("])");
    out
}

/// Build the JS eval string that dispatches an event to frontend subscribers.
#[inline]
pub fn eval_dispatch(name: &str, payload: &Value) -> String {
    let name_js = serde_json::to_string(name).unwrap_or_else(|_| "\"\"".into());
    let payload_js = serde_json::to_string(payload).unwrap_or_else(|_| "null".into());
    format!("window.__tynd_dispatch__&&window.__tynd_dispatch__({name_js},{payload_js})")
}

/// Build the JS eval string that resolves/rejects a pending OS API call.
#[inline]
pub fn eval_os_result(id: &str, ok: bool, value: &Value) -> String {
    let ok_js = if ok { "true" } else { "false" };
    let val_js = serde_json::to_string(value).unwrap_or_else(|_| "null".into());
    let id_js = serde_json::to_string(id).unwrap_or_else(|_| "\"\"".into());
    format!("window.__tynd_os_result__&&window.__tynd_os_result__({id_js},{ok_js},{val_js})")
}

/// Build the JS eval string that fires a native OS event to subscribed handlers.
#[inline]
pub fn eval_os_event(name: &str, data: &Value) -> String {
    let name_js = serde_json::to_string(name).unwrap_or_else(|_| "\"\"".into());
    let data_js = serde_json::to_string(data).unwrap_or_else(|_| "null".into());
    format!("window.__tynd_os_event__&&window.__tynd_os_event__({name_js},{data_js})")
}

/// Build the JS eval string that mounts a full-screen backend error overlay.
/// Dev-only — called when the backend throws during startup or reload.
#[inline]
pub fn eval_show_error_overlay(message: &str) -> String {
    let msg_js = serde_json::to_string(message).unwrap_or_else(|_| "\"\"".into());
    format!(
        "(function(){{var id='__tynd_error_overlay__';var old=document.getElementById(id);if(old)old.remove();\
var d=document.createElement('div');d.id=id;\
d.style.cssText='position:fixed;inset:0;z-index:2147483647;background:rgba(20,20,20,0.92);color:#fce5e5;\
font:14px/1.5 ui-monospace,SFMono-Regular,Menlo,Consolas,monospace;padding:32px;overflow:auto;white-space:pre-wrap;';\
d.textContent='\u{26a0}  Backend error\\n\\n'+{msg_js};document.body?document.body.appendChild(d):document.documentElement.appendChild(d);}})();"
    )
}

/// Build the JS eval string that removes the backend error overlay (on successful reload).
#[inline]
pub fn eval_hide_error_overlay() -> String {
    "(function(){var o=document.getElementById('__tynd_error_overlay__');if(o)o.remove();})();"
        .to_string()
}
