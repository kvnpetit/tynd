// QuickJS globals installed *before* the user bundle runs. Provides the
// bare minimum the rest of the backend (polyfills + user code) expects:
//   - `console.log/info/warn/error/debug` routed to the host via __tynd_log__
//   - `setTimeout` / `setInterval` / `clearTimeout` / `clearInterval`
//     backed by a single Rust timer thread (see quickjs::timer)
//   - `__tynd_mod__` = globalThis (so exports land where the call dispatcher
//     looks for them)
//   - `__tynd_call__(id, fn, argsJson)` RPC dispatch with async-iterable
//     streaming + `__tynd_cancel__(id)` for stopping an active stream
//
// Polyfills (fetch, crypto, URL, ...) are evaluated *after* this file by
// `polyfills::install`, and may replace parts of this (e.g. `console` gets
// extended with group/table/etc in polyfills/console-ext.js).

globalThis.globalThis = globalThis;
(function () {
  function fmt(a) {
    try { return typeof a === 'string' ? a : JSON.stringify(a); }
    catch (_) { return String(a); }
  }
  function join(args) {
    var out = '';
    for (var i = 0; i < args.length; i++) { if (i) out += ' '; out += fmt(args[i]); }
    return out;
  }
  globalThis.console = {
    log:   function () { __tynd_log__('log',   join(arguments)); },
    info:  function () { __tynd_log__('info',  join(arguments)); },
    warn:  function () { __tynd_log__('warn',  join(arguments)); },
    error: function () { __tynd_log__('error', join(arguments)); },
    debug: function () { __tynd_log__('debug', join(arguments)); },
  };
})();

(function () {
  var nextId = 1;
  var handlers = {};
  globalThis.setTimeout = function (fn, ms) {
    var id = nextId++; handlers[id] = { fn: fn, once: true };
    __tynd_set_interval__(id, ms || 0, true);
    return id;
  };
  globalThis.setInterval = function (fn, ms) {
    var id = nextId++; handlers[id] = { fn: fn, once: false };
    __tynd_set_interval__(id, ms || 0, false);
    return id;
  };
  globalThis.clearTimeout = function (id) {
    delete handlers[id]; __tynd_clear_interval__(id);
  };
  globalThis.clearInterval = globalThis.clearTimeout;
  // Fired by Rust timer thread
  globalThis.__tynd_fire_timer__ = function (id) {
    var h = handlers[id]; if (!h) return;
    if (h.once) delete handlers[id];
    try { h.fn(); } catch (e) { console.error('timer ' + id + ': ' + e); }
  };
})();

// Store user module exports on globalThis for RPC dispatch
globalThis.__tynd_mod__ = globalThis;

// RPC dispatch + streaming helper. Invoked by Rust via __tynd_call__(id, fn, argsJson).
// Handles:
//   - plain return values and Promises (-> single __tynd_return__)
//   - async iterables (-> __tynd_yield__ per chunk, __tynd_return__ at end with
//     the generator's return value).
// Cancellation: __tynd_cancel__(id) calls iterator.return() on the active stream.
(function () {
  var streams = {};
  function stringify(v) { try { return JSON.stringify(v === undefined ? null : v); } catch (_) { return 'null'; } }
  function isAsyncIterable(x) {
    return x != null && (typeof x === 'object' || typeof x === 'function')
      && typeof x[Symbol.asyncIterator] === 'function';
  }
  function runStream(id, iter) {
    streams[id] = iter;
    function step() {
      Promise.resolve(iter.next()).then(function (s) {
        if (!streams[id]) return; // cancelled
        if (s.done) {
          delete streams[id];
          __tynd_return__(id, true, stringify(s.value));
          return;
        }
        __tynd_yield__(id, stringify(s.value));
        step();
      }, function (err) {
        delete streams[id];
        __tynd_return__(id, false, stringify(String(err && err.message || err)));
      });
    }
    step();
  }
  globalThis.__tynd_call__ = function (id, fnName, argsJson) {
    var mod = globalThis.__tynd_mod__;
    var fn = mod && mod[fnName];
    if (typeof fn !== 'function') {
      __tynd_return__(id, false, stringify('Unknown function: ' + fnName));
      return;
    }
    var args;
    try { args = JSON.parse(argsJson); } catch (_) { args = []; }
    var result;
    try { result = fn.apply(mod, args); }
    catch (e) { __tynd_return__(id, false, stringify(String(e && e.message || e))); return; }
    Promise.resolve(result).then(function (r) {
      if (isAsyncIterable(r)) {
        runStream(id, r[Symbol.asyncIterator]());
      } else {
        __tynd_return__(id, true, stringify(r));
      }
    }, function (err) {
      __tynd_return__(id, false, stringify(String(err && err.message || err)));
    });
  };
  globalThis.__tynd_cancel__ = function (id) {
    var iter = streams[id];
    if (!iter) return;
    delete streams[id];
    try {
      if (typeof iter['return'] === 'function') {
        Promise.resolve(iter['return']()).catch(function () {});
      }
    } catch (_) {}
    __tynd_return__(id, true, 'null');
  };
})();
