//! JavaScript injected into every webview (IPC shim, devalue bundle, error
//! overlay) and the helpers that build the `evaluate_script` calls feeding it.

use serde_json::Value;

pub(crate) mod tokens;

/// Devalue bundle (~8 KB) — exposes `globalThis.__tynd_dv__ = { parse,
/// stringify }` so Date/Map/Set/BigInt/URL/Error/cyclic refs round-trip
/// across IPC. Regenerate via `bun run build:devalue` in core.
pub(crate) const JS_DEVALUE: &str = include_str!("shim/devalue.bundle.js");

/// Frameless-window helpers: `data-tynd-drag` / `data-tynd-no-drag` drag
/// regions (double-click maximizes), `data-tynd-resize` handles and the
/// automatic resize border of frameless windows. Runs after [`JS_SHIM`].
pub(crate) const JS_FRAME: &str = include_str!("shim/frame.js");

/// Injected only in debug mode (`--debug` / `tynd dev`).
/// Sets `window.__tynd_dev__ = true` so framework JS can prefix its own
/// console messages with `[tynd]` without leaking the name in production.
pub(crate) const JS_DEV_FLAG: &str = "window.__tynd_dev__ = true;";

/// Core IPC shim — creates window.__tynd__ with call/on/off/os_call and
/// sends the page-ready signal. A template: [`ipc_shim`] fills in the
/// webview's IPC token and the app origins a subframe must have to get the
/// bridge. Injected before any page script via with_initialization_script.
///
/// Wire format: every user payload (`call.args`, `return.value`,
/// `yield.value`, `event.payload`) rides as a string. Producers encode
/// with devalue (Bun runtime — Date/Map/Set/BigInt/cycles roundtrip)
/// or plain JSON. The shim's `_decode` tries devalue
/// first then falls back to JSON, so both producers work transparently.
/// `os_call.args` deliberately stays as a JS value because the Rust
/// host inspects fields like `url` / `width` natively.
pub(crate) const JS_SHIM: &str = r#"
(function () {
  "use strict";

  // The bridge belongs to the top frame. A subframe gets it only when its own
  // location is app content; any other frame (a remote iframe, an opaque
  // `srcdoc` / `data:` frame) gets nothing. WebView2 runs init scripts in
  // every frame; WebKit runs them in the top frame only.
  var _appOrigins = [/*__TYND_APP_ORIGINS__*/];
  var _loc = window.location;
  function _isAppLocation(loc) {
    return !!loc && (loc.protocol === "tynd:" || _appOrigins.indexOf(loc.origin) >= 0);
  }
  var _isTop = window.top === window;
  if (!_isTop && !_isAppLocation(_loc)) return;
  // App mode: this is the top document and it is app content (`about:blank`
  // is the placeholder page or a `loadHtml` document). Decided once, here,
  // before any page script runs; `location` cannot be spoofed.
  var _appMode = _isTop && (_isAppLocation(_loc) || (!!_loc && _loc.href === "about:blank"));

  // Every IPC message starts with one of this webview's two secret tokens:
  // the host drops messages with neither.
  // - The remote token is baked into this script, which every document of
  //   the webview receives, remote ones included. It never earns more than
  //   remote-origin privileges.
  // - The app token is never part of this script. An app document asks the
  //   host for it at start-up (a remote-token message `?<request id>`); the
  //   host answers only while the main frame shows app content, by
  //   evaluating `__tynd_os_result__(<request id>, true, <token>)` in the
  //   main frame, so a remote document never holds it. Messages posted
  //   before the answer are queued.
  // Tokens live only in this closure, so a cross-origin frame that calls the
  // raw platform message handler cannot forge a call. The sink and
  // `Reflect.apply` are captured before any page script runs, so later page
  // code cannot observe a token by patching them.
  var _remoteToken = "__TYND_IPC_TOKEN__";
  var _token = _appMode ? null : _remoteToken;
  var _queue = [];
  var _appTokenId = null;
  if (_appMode) {
    var _rand;
    try { _rand = crypto.randomUUID(); } catch (e) { _rand = String(Date.now()) + "-" + String(Math.random()).slice(2); }
    _appTokenId = "tynd-app-token:" + _rand;
  }
  var _apply = Reflect.apply;
  var _sink = (function () {
    var wk = window.webkit;
    if (wk && wk.messageHandlers && wk.messageHandlers.ipc) return wk.messageHandlers.ipc;
    var cw = window.chrome && window.chrome.webview;
    if (cw && typeof cw.postMessage === "function") return cw;
    return window.ipc;
  })();
  var _send = _sink && _sink.postMessage;
  function _post(message) {
    if (typeof _send !== "function") throw new Error("IPC channel unavailable");
    if (_token === null) {
      _queue.push(message);
      return;
    }
    _apply(_send, _sink, [_token + message]);
  }
  function _receiveAppToken(token) {
    if (_token !== null || typeof token !== "string" || token.length === 0) return;
    _token = token;
    var queued = _queue;
    _queue = [];
    for (var i = 0; i < queued.length; i++) _apply(_send, _sink, [_token + queued[i]]);
  }

  // Page-ready signal on DOMContentLoaded (skips the initial blank page).
  // Rust intercepts it to show the window only after real content is
  // rendered, eliminating the white flash on startup.
  if (typeof document !== "undefined") {
    document.addEventListener("DOMContentLoaded", function () {
      var url = document.URL;
      if (url && url !== "about:blank" && url.indexOf("data:") !== 0) {
        _post(JSON.stringify({ __tynd_page_ready: true }));
      }
    });
  }

  // Call IDs use `crypto.randomUUID()` (UUIDv4). Backend-side ids are v7
  // via `uuid::Uuid::now_v7`; swap the shim to v7 once a polyfill matures.
  var _pending = Object.create(null);
  var _os_pending = Object.create(null);
  var _listeners = Object.create(null);
  var _os_listeners = Object.create(null);
  var _backend_dead_error = null;
  // Internal logger: prefixes with [tynd] in dev mode only (window.__tynd_dev__ injected by tynd dev)
  var _log = window.__tynd_dev__
    ? function(msg) { console.error("[tynd] " + msg); }
    : function() {};

  // Decode a wire payload — devalue first (Bun runtime), JSON fallback
  // (plain values that happen to round-trip through JSON).
  // Returns `null` on double failure so handlers don't see `undefined`.
  function _decode(s) {
    if (s == null) return null;
    var dv = window.__tynd_dv__;
    if (dv && typeof dv.parse === "function") {
      try { return dv.parse(s); } catch (e) { _log("ipc decode (devalue) failed: " + e); }
    }
    try { return JSON.parse(s); } catch (e) { _log("ipc decode (json) failed: " + e); return null; }
  }

  // Encode for the wire — devalue when available, JSON otherwise. The
  // call() / os fallback paths use this when sending args back to Rust.
  function _encode(v) {
    var dv = window.__tynd_dv__;
    if (dv && typeof dv.stringify === "function") {
      try { return dv.stringify(v); } catch (e) { _log("ipc encode (devalue) failed: " + e); }
    }
    try { return JSON.stringify(v); } catch (e) { _log("ipc encode (json) failed: " + e); return "null"; }
  }

  // Called by Rust: resolve/reject a pending backend call promise + close its stream.
  // `valueStr` is a wire-encoded payload — devalue or JSON. On error,
  // `valueStr` is the plain error message text (no encoding).
  window.__tynd_resolve__ = function (id, ok, valueStr) {
    var p = _pending[id];
    if (!p) return;
    delete _pending[id];
    if (ok) {
      var value = _decode(valueStr);
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
      var errorText = String(valueStr);
      p.error = new Error(errorText);
      if (errorText.indexOf("BACKEND_DEAD:") === 0) {
        p.error.code = "BACKEND_DEAD";
      }
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
        _post(JSON.stringify({ type: "ack", id: id, n: p.consumed }));
      } catch (e) {
        _log("ipc ack postMessage failed (window=" + (window.__TYND_WINDOW_LABEL__ || "?") + "): " + e);
      }
      p.consumed = 0;
    }
  }

  // Called by Rust: deliver one chunk from a streaming backend call.
  // `valueStr` is wire-encoded (devalue or JSON) — decoded once here.
  window.__tynd_yield__ = function (id, valueStr) {
    var p = _pending[id];
    if (!p || p.done) return;
    var value = _decode(valueStr);
    if (p.waiters.length) {
      p.waiters.shift().resolve({ value: value, done: false });
      _ackConsume(id, p);
    } else {
      p.yields.push(value);
    }
  };

  // Called by Rust: deliver a batch of yields in one call to minimize
  // `evaluate_script` overhead on bursty streams. `pairs` is an array of
  // [id, valueStr] tuples in FIFO order.
  window.__tynd_yield_batch__ = function (pairs) {
    for (var i = 0; i < pairs.length; i++) {
      window.__tynd_yield__(pairs[i][0], pairs[i][1]);
    }
  };

  // Called by Rust: resolve/reject a pending OS API call promise, or hand
  // this app document its app token. Not writable: a remote document must
  // not be able to install its own receiver for a token answer that lands
  // in it after a navigation.
  Object.defineProperty(window, "__tynd_os_result__", {
    value: function (id, ok, value) {
      if (_appTokenId !== null && id === _appTokenId) {
        if (ok) _receiveAppToken(value);
        return;
      }
      var p = _os_pending[id];
      if (!p) return;
      delete _os_pending[id];
      if (ok) { p.resolve(value); } else { p.reject(new Error(String(value))); }
    },
    writable: false,
    configurable: false,
    enumerable: false,
  });

  // Called by Rust when the Bun child dies unexpectedly. Rejects every
  // in-flight `__tynd__.call` and `os_call` so awaiting code sees a
  // BACKEND_DEAD error instead of hanging on a Promise the host can no
  // longer settle.
  window.__tynd_backend_dead__ = function (message) {
    var err = new Error("BACKEND_DEAD: " + String(message || "backend exited"));
    err.code = "BACKEND_DEAD";
    _backend_dead_error = err;
    for (var k in _pending) {
      var e = _pending[k];
      if (!e || e.done) continue;
      e.done = true;
      e.error = err;
      try { e.reject(err); } catch (_) {}
      var ws = e.waiters || [];
      for (var i = 0; i < ws.length; i++) {
        try { ws[i].reject(err); } catch (_) {}
      }
      e.waiters = [];
      delete _pending[k];
    }
    for (var k2 in _os_pending) {
      var oe = _os_pending[k2];
      if (!oe) continue;
      try { oe.reject(err); } catch (_) {}
      delete _os_pending[k2];
    }
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

  // Called by Rust: dispatch a named event to frontend subscribers.
  // `payloadStr` is wire-encoded — decoded once here so each handler
  // sees a live JS value (Date / Map / Set roundtrip if devalue path).
  window.__tynd_dispatch__ = function (name, payloadStr) {
    var handlers = _listeners[name];
    if (!handlers) return;
    var payload = _decode(payloadStr);
    for (var i = 0; i < handlers.length; i++) {
      try { handlers[i](payload); } catch (e) {
        _log("event handler error: " + e);
      }
    }
  };

  var bridge = {
    // Call a backend function — returns an awaitable+iterable handle.
    // `await call(...)` -> final return value (regular async handlers, or
    // the generator's `return` value for async-generators).
    // `for await (x of call(...))` -> yielded chunks.
    call: function (fn, args) {
      var id = crypto.randomUUID();
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
              _post(JSON.stringify({ type: "cancel", id: id }));
            } catch (e) {
              _log("ipc cancel postMessage failed (window=" + (window.__TYND_WINDOW_LABEL__ || "?") + "): " + e);
            }
            // The host drops its call label on cancel and the backend sends
            // no `return` frame for a cancelled call, so settle locally:
            // free the entry, reject the final promise and every parked
            // `next()` with a CANCELLED envelope. The final promise is marked
            // handled first: `break` out of a `for await` never awaits it.
            delete _pending[id];
            p.done = true;
            p.yields = [];
            p.error = new Error(
              "__TYND_IPC_ERR__:" +
                JSON.stringify({ name: "TyndIpcError", code: "CANCELLED", message: "call cancelled", fn: fn })
            );
            p.error.code = "CANCELLED";
            finalPromise["catch"](function () {});
            p.reject(p.error);
            var ws = p.waiters;
            p.waiters = [];
            for (var i = 0; i < ws.length; i++) ws[i].reject(p.error);
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

      // Keep failure sticky until a successful reload replaces this document
      // and reinstalls the shim. Calls made after the crash must reject
      // immediately instead of entering `_pending` with no live Bun reader.
      if (_backend_dead_error) {
        delete _pending[id];
        p.done = true;
        p.error = _backend_dead_error;
        p.reject(p.error);
        return handle;
      }

      // Encode args via devalue (or JSON fallback) — Bun decodes the
      // same way before invoking the user's handler. The `a` field is
      // a string literal in the wire frame so the Rust shuttle can
      // forward it without parsing.
      try {
        _post(
          JSON.stringify({ type: "call", id: id, fn: fn, a: _encode(args) })
        );
      } catch (e) {
        delete _pending[id];
        p.done = true;
        p.error = e instanceof Error ? e : new Error(String(e));
        p.reject(p.error);
      }
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

    // Call a native OS API — returns a Promise. There is no default
    // deadline: dialogs wait on the user and downloads / process runs can
    // legitimately take minutes. A caller that wants a wedge guard passes
    // `opts.timeoutMs` (a positive number of milliseconds).
    os_call: function (api, method, args, opts) {
      return new Promise(function (resolve, reject) {
        var id = "os_" + crypto.randomUUID();
        var timeoutMs = opts && typeof opts.timeoutMs === "number" && opts.timeoutMs > 0 ? opts.timeoutMs : 0;
        var timer = timeoutMs
          ? setTimeout(function () {
              if (_os_pending[id]) {
                delete _os_pending[id];
                reject(new Error("os_call(" + api + "." + method + ") timed out after " + timeoutMs + "ms"));
              }
            }, timeoutMs)
          : 0;
        _os_pending[id] = {
          resolve: function (v) { clearTimeout(timer); resolve(v); },
          reject: function (e) { clearTimeout(timer); reject(e); },
        };
        try {
          _post(
            JSON.stringify({ type: "os_call", id: id, api: api, method: method, args: args })
          );
        } catch (e) {
          clearTimeout(timer);
          delete _os_pending[id];
          reject(e instanceof Error ? e : new Error(String(e)));
        }
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

  // The bridge cannot be replaced or patched by later page scripts: framework
  // code (context menus, frame helpers) and plugins read this one object.
  // Authorization never depends on it: the host checks the calling frame's
  // origin on every IPC message.
  Object.defineProperty(window, "__tynd__", {
    value: Object.freeze(bridge),
    writable: false,
    configurable: false,
    enumerable: false,
  });

  // Ask for the app token last, once `__tynd_os_result__` can receive it.
  if (_appMode && typeof _send === "function") {
    _apply(_send, _sink, [_remoteToken + "?" + _appTokenId]);
  }
})();
"#;

/// Quoted JS literal of the IPC token in [`JS_SHIM`].
const TOKEN_PLACEHOLDER: &str = "\"__TYND_IPC_TOKEN__\"";
/// Body of the app-origin array literal in [`JS_SHIM`].
const APP_ORIGINS_PLACEHOLDER: &str = "/*__TYND_APP_ORIGINS__*/";

/// Length in hex characters of a webview IPC token (256 random bits).
const IPC_TOKEN_LEN: usize = 64;

/// A fresh per-webview IPC token from the OS CSPRNG, hex-encoded.
pub(crate) fn new_ipc_token() -> String {
    use std::fmt::Write as _;
    let mut bytes = [0u8; IPC_TOKEN_LEN / 2];
    // OS CSPRNG failure is unrecoverable and practically unreachable.
    getrandom::fill(&mut bytes).expect("OS CSPRNG unavailable");
    bytes
        .iter()
        .fold(String::with_capacity(IPC_TOKEN_LEN), |mut out, byte| {
            let _ = write!(out, "{byte:02x}");
            out
        })
}

/// JS string literals of the non-`tynd:` app origins a document may have:
/// the WebView2 spelling of `tynd:` (Windows) and the dev server.
fn app_origin_literals(dev_origin: Option<&str>) -> Vec<String> {
    let mut origins = Vec::new();
    if cfg!(target_os = "windows") {
        origins.push(js_str("http://tynd.localhost"));
    }
    if let Some(origin) = dev_origin {
        origins.push(js_str(origin));
    }
    origins
}

/// [`JS_SHIM`] for one webview: `remote_token` is the token baked into the
/// script (see [`tokens`] for the app token), and a subframe installs the
/// bridge only when its own origin is app content (`tynd:`, the WebView2
/// spelling of it, or the dev server `dev_origin`).
pub(crate) fn ipc_shim(remote_token: &str, dev_origin: Option<&str>) -> String {
    JS_SHIM
        .replace(TOKEN_PLACEHOLDER, &js_str(remote_token))
        .replace(
            APP_ORIGINS_PLACEHOLDER,
            &app_origin_literals(dev_origin).join(","),
        )
}

/// Strip the IPC token from the front of a message body. Returns `false`
/// (leaving `body` untouched) when the body does not start with `token`: the
/// message did not come from this webview's shim (a cross-origin frame calling
/// the raw platform handler) and must be dropped.
pub(crate) fn take_ipc_token(body: &mut String, token: &str) -> bool {
    debug_assert!(!token.is_empty(), "the IPC token is never empty");
    let Some(prefix) = body.as_bytes().get(..token.len()) else {
        return false;
    };
    // Constant-time comparison: the mismatch position must not leak.
    let diff = prefix
        .iter()
        .zip(token.as_bytes())
        .fold(0u8, |acc, (a, b)| acc | (a ^ b));
    if token.is_empty() || diff != 0 {
        return false;
    }
    body.drain(..token.len());
    true
}

/// Replace U+2028 (LINE SEPARATOR) and U+2029 (PARAGRAPH SEPARATOR) with
/// their `\u` escapes inside an already-serialized JSON literal.
///
/// JSON treats them as ordinary characters, but a JS source-text parser
/// treats them as line terminators that prematurely close any embedding
/// `eval(...)` literal. Without the swap, a JSON payload containing those
/// code points could break out of its string literal, letting a frontend
/// smuggle JS into the host's eval surface.
#[inline]
fn js_escape_line_terminators(encoded: String) -> String {
    if encoded.contains('\u{2028}') || encoded.contains('\u{2029}') {
        encoded
            .replace('\u{2028}', "\\u2028")
            .replace('\u{2029}', "\\u2029")
    } else {
        encoded
    }
}

/// JSON-encode a `&str` for use as a JS literal. Falls back to an empty
/// string literal on the (impossible) serde failure so callers stay total.
#[inline]
fn js_str(s: &str) -> String {
    let encoded = serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into());
    js_escape_line_terminators(encoded)
}

/// JSON-encode a `serde_json::Value` for use as a JS literal. Falls back
/// to `null` on the (impossible) serde failure so callers stay total.
/// Applies the same U+2028/U+2029 escape as `js_str` so embedded strings
/// inside the value cannot terminate the surrounding JS literal.
#[inline]
pub(crate) fn js_val(v: &Value) -> String {
    let encoded = serde_json::to_string(v).unwrap_or_else(|_| "null".into());
    js_escape_line_terminators(encoded)
}

/// Build a defensive `window.__tynd_<fn>__` invocation: guards the symbol
/// with `&&` so the call is a noop when the shim hasn't loaded yet (race
/// at startup), then forwards the formatted argument list. Every
/// `eval_*` builder below threads through this single shape so adding a
/// new wire entry-point is a one-liner.
macro_rules! eval_call {
    ($fn:literal, $args:expr) => {
        format!(concat!("window.", $fn, "&&window.", $fn, "({})"), $args)
    };
}

/// Build the JS eval string that resolves/rejects a pending frontend call.
///
/// `value` is a wire-encoded payload string (devalue or plain JSON). On
/// error it carries the plain message. The shim's
/// `__tynd_resolve__` decodes once before resolving the user's promise.
#[inline]
pub(crate) fn eval_resolve(id: &str, ok: bool, value: &str) -> String {
    let ok_js = if ok { "true" } else { "false" };
    eval_call!(
        "__tynd_resolve__",
        format_args!("{},{ok_js},{}", js_str(id), js_str(value))
    )
}

/// Build the JS eval string that delivers one chunk from a streaming backend call.
/// `value` is the wire-encoded payload string.
#[inline]
pub(crate) fn eval_yield(id: &str, value: &str) -> String {
    eval_call!(
        "__tynd_yield__",
        format_args!("{},{}", js_str(id), js_str(value))
    )
}

/// Batched yield flush. Renders to `__tynd_yield_batch__([[id,val],…])` so
/// bursty streams hit one `evaluate_script` instead of one per yield.
/// Caches the encoded id across consecutive same-id entries to skip a
/// redundant `serde_json::to_string` per chunk.
#[inline]
pub(crate) fn eval_yield_batch(pairs: &[(String, String)]) -> String {
    // Build directly into a `Vec<u8>` to avoid repeated String reallocations.
    let mut out = Vec::<u8>::with_capacity(pairs.len() * 48);
    out.extend_from_slice(b"window.__tynd_yield_batch__&&window.__tynd_yield_batch__([");
    let mut last_id: Option<&str> = None;
    let mut last_id_encoded: String = String::new();
    for (i, (id, value)) in pairs.iter().enumerate() {
        if i > 0 {
            out.push(b',');
        }
        out.push(b'[');
        // Cache JSON-encoded id across consecutive same-id yields.
        if last_id.map(str::as_bytes) != Some(id.as_bytes()) {
            last_id_encoded = js_str(id);
            last_id = Some(id.as_str());
        }
        out.extend_from_slice(last_id_encoded.as_bytes());
        out.push(b',');
        out.extend_from_slice(js_str(value).as_bytes());
        out.push(b']');
    }
    out.extend_from_slice(b"])");
    String::from_utf8(out).unwrap_or_default()
}

/// Build the JS eval string that dispatches an event to frontend subscribers.
/// `payload` is the wire-encoded payload string.
#[inline]
pub(crate) fn eval_dispatch(name: &str, payload: &str) -> String {
    eval_call!(
        "__tynd_dispatch__",
        format_args!("{},{}", js_str(name), js_str(payload))
    )
}

/// Build the JS eval string that resolves/rejects a pending OS API call.
#[inline]
///
/// The answer to an app-token request ([`tokens::APP_TOKEN_ID_PREFIX`]) is
/// wrapped in a check of the main frame's own `location`, so it is a no-op in
/// a remote document that replaced the requesting app document in between.
pub(crate) fn eval_os_result(id: &str, ok: bool, value: &Value) -> String {
    let ok_js = if ok { "true" } else { "false" };
    let call = eval_call!(
        "__tynd_os_result__",
        format_args!("{},{ok_js},{}", js_str(id), js_val(value))
    );
    if id.starts_with(tokens::APP_TOKEN_ID_PREFIX) {
        app_location_guard(&call, crate::webview::navigation::dev_origin().as_deref())
    } else {
        call
    }
}

/// Run `script` only when the evaluating document is app content. Only
/// strict comparisons on the unforgeable `window.location`: a remote page
/// can patch `Array.prototype.indexOf`, not these.
fn app_location_guard(script: &str, dev_origin: Option<&str>) -> String {
    use std::fmt::Write as _;
    let mut test = String::from("l.protocol===\"tynd:\"||l.href===\"about:blank\"");
    for origin in app_origin_literals(dev_origin) {
        let _ = write!(test, "||l.origin==={origin}");
    }
    format!("(function(l){{if(l&&({test})){{{script}}}}})(window.location)")
}

/// Build the JS eval string that fires a native OS event to subscribed handlers.
#[inline]
pub(crate) fn eval_os_event(name: &str, data: &Value) -> String {
    eval_call!(
        "__tynd_os_event__",
        format_args!("{},{}", js_str(name), js_val(data))
    )
}

/// Source of the dev-mode backend error overlay. The `__TYND_OVERLAY_PAYLOAD__`
/// placeholder is substituted at call time with the JSON-encoded error
/// message before the script ships to the webview.
const JS_ERROR_OVERLAY: &str = include_str!("shim/error_overlay.js");

/// Build the JS eval string that mounts a full-screen backend error overlay.
/// Dev-only — called when the backend throws during startup or reload.
#[inline]
pub(crate) fn eval_show_error_overlay(message: &str) -> String {
    let payload = serde_json::json!({
        "code": "BACKEND_ERROR",
        "title": "Backend error",
        "message": message.lines().next().unwrap_or(message),
        "stack": message,
        "reportCommand": "tynd report",
        "fixCommand": "tynd doctor --fix --dry-run",
        "explainCommand": "tynd explain BACKEND_ERROR",
    });
    JS_ERROR_OVERLAY.replace("__TYND_OVERLAY_PAYLOAD__", &js_val(&payload))
}

/// Build the JS eval string that removes the backend error overlay (on successful reload).
#[inline]
pub(crate) fn eval_hide_error_overlay() -> String {
    "(function(){var o=document.getElementById('__tynd_error_overlay__');if(o)o.remove();})();"
        .to_string()
}

/// Reject every pending `__tynd__.call` and `os_call` with a BACKEND_DEAD
/// error so awaiting code unblocks instead of hanging forever after a Bun
/// child crash. Idempotent — safe to evaluate twice; second pass finds
/// the maps empty.
pub(crate) fn eval_backend_dead(message: &str) -> String {
    format!(
        "if(window.__tynd_backend_dead__)window.__tynd_backend_dead__({});",
        js_str(message),
    )
}

#[cfg(test)]
mod tests;
