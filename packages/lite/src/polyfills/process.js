// Node `process` global + node:process module.
//
// Pure JS surface backed by tiny Rust bridges (see polyfills::process_rs):
//   __tynd_platform__()    -> "win32" | "darwin" | "linux"
//   __tynd_arch__()        -> "x64"  | "arm64" | ...
//   __tynd_pid__()         -> u32
//   __tynd_cwd__()         -> string
//   __tynd_chdir__(path)   -> void  (throws on err)
//   __tynd_env_get__(k)    -> string|null
//   __tynd_env_keys__()    -> string[]
//   __tynd_env_set__(k,v)  -> void
//   __tynd_env_del__(k)    -> void
//   __tynd_argv__()        -> string[] (executable + flags)
//   __tynd_exit__(code)    -> void (host shutdown)
//   __tynd_hrtime_ms__()   -> number (high-resolution monotonic ms)
//   __tynd_uptime_ms__()   -> number (process uptime ms)
//
// We intentionally don't model stdin/stdout/stderr as full streams here:
// expose `.write` shims that route to console; libs that pipe into stdout
// will get correct text but no real backpressure (tynd backends aren't
// CLIs). For full CLI tooling, see tynd-cli's own runtime.
(function () {
  var ENV = new Proxy({}, {
    get: function (_t, key) {
      if (typeof key !== "string") return undefined;
      var v = (typeof __tynd_env_get__ === "function") ? __tynd_env_get__(key) : null;
      return v == null ? undefined : v;
    },
    set: function (_t, key, value) {
      if (typeof key !== "string") return true;
      if (typeof __tynd_env_set__ === "function") __tynd_env_set__(key, String(value));
      return true;
    },
    deleteProperty: function (_t, key) {
      if (typeof key !== "string") return true;
      if (typeof __tynd_env_del__ === "function") __tynd_env_del__(key);
      return true;
    },
    has: function (_t, key) {
      if (typeof key !== "string") return false;
      var v = (typeof __tynd_env_get__ === "function") ? __tynd_env_get__(key) : null;
      return v != null;
    },
    ownKeys: function () {
      return (typeof __tynd_env_keys__ === "function") ? __tynd_env_keys__() : [];
    },
    getOwnPropertyDescriptor: function (_t, key) {
      if (typeof key !== "string") return undefined;
      var v = (typeof __tynd_env_get__ === "function") ? __tynd_env_get__(key) : null;
      if (v == null) return undefined;
      return { value: v, writable: true, enumerable: true, configurable: true };
    },
  });

  function _platform() {
    if (typeof __tynd_platform__ === "function") return __tynd_platform__();
    return "linux";
  }
  function _arch() {
    if (typeof __tynd_arch__ === "function") return __tynd_arch__();
    return "x64";
  }

  var listeners = Object.create(null);
  function on(ev, fn) { (listeners[ev] = listeners[ev] || []).push(fn); return process; }
  function off(ev, fn) {
    var arr = listeners[ev]; if (!arr) return process;
    var i = arr.indexOf(fn); if (i >= 0) arr.splice(i, 1);
    return process;
  }
  function emit(ev) {
    var arr = listeners[ev]; if (!arr) return false;
    var args = Array.prototype.slice.call(arguments, 1);
    var snap = arr.slice();
    for (var i = 0; i < snap.length; i++) { try { snap[i].apply(process, args); } catch (e) { console.error(e); } }
    return true;
  }

  function nextTick(fn) {
    var args = Array.prototype.slice.call(arguments, 1);
    Promise.resolve().then(function () { try { fn.apply(null, args); } catch (e) { console.error(e); } });
  }

  // Minimal stdout/stderr — write text via console; .isTTY=false so libs
  // skip ANSI / cursor magic that wouldn't make sense in a backend.
  function makeStream(level) {
    return {
      isTTY: false,
      write: function (chunk) {
        try {
          var s = typeof chunk === "string" ? chunk : (chunk instanceof Uint8Array ? new TextDecoder().decode(chunk) : String(chunk));
          if (level === "error") console.error(s); else console.log(s);
        } catch (_) {}
        return true;
      },
      end: function () { return true; },
      on: function () { return this; },
      once: function () { return this; },
      removeListener: function () { return this; },
      columns: 80,
      rows: 24,
      fd: level === "error" ? 2 : 1,
    };
  }

  var startMs = (typeof __tynd_uptime_ms__ === "function") ? __tynd_uptime_ms__() : 0;

  var process = {
    env: ENV,
    argv: (typeof __tynd_argv__ === "function") ? __tynd_argv__() : ["tynd-lite"],
    argv0: "tynd-lite",
    execPath: (typeof __tynd_argv__ === "function") ? (__tynd_argv__()[0] || "tynd-lite") : "tynd-lite",
    execArgv: [],
    pid: (typeof __tynd_pid__ === "function") ? __tynd_pid__() : 0,
    ppid: 0,
    title: "tynd-lite",
    arch: _arch(),
    platform: _platform(),
    versions: { tynd: "1.0.0", quickjs: "0.6", v8: "0.0.0", node: "20.0.0", openssl: "0.0.0" },
    version: "v20.0.0",
    release: { name: "node" },
    features: { ipv6: true },
    config: {},
    debugPort: 0,
    cwd: function () { return (typeof __tynd_cwd__ === "function") ? __tynd_cwd__() : "/"; },
    chdir: function (dir) {
      if (typeof __tynd_chdir__ !== "function") throw new Error("process.chdir: unavailable in this build");
      __tynd_chdir__(String(dir));
    },
    exit: function (code) {
      try { emit("exit", code | 0); } catch (_) {}
      if (typeof __tynd_exit__ === "function") __tynd_exit__(code | 0);
      else throw new Error("process.exit(" + code + ") — no exit bridge");
    },
    abort: function () { this.exit(134); },
    kill: function (_pid, _sig) { return true; },
    nextTick: nextTick,
    hrtime: function (prev) {
      var now = (typeof __tynd_hrtime_ms__ === "function") ? __tynd_hrtime_ms__() : Date.now();
      var seconds = Math.floor(now / 1000);
      var nanos = Math.floor((now - seconds * 1000) * 1e6);
      if (Array.isArray(prev) && prev.length === 2) {
        var ds = seconds - prev[0];
        var dn = nanos - prev[1];
        if (dn < 0) { ds--; dn += 1e9; }
        return [ds, dn];
      }
      return [seconds, nanos];
    },
    uptime: function () {
      var n = (typeof __tynd_uptime_ms__ === "function") ? __tynd_uptime_ms__() : Date.now();
      return Math.max(0, (n - startMs) / 1000);
    },
    memoryUsage: function () {
      // QuickJS has no introspection bridge — return zeros so libs that
      // log this don't crash. Plugins needing real memory go via
      // `os.system_stats` from `@tynd/core/client`.
      return { rss: 0, heapTotal: 0, heapUsed: 0, external: 0, arrayBuffers: 0 };
    },
    cpuUsage: function () { return { user: 0, system: 0 }; },
    on: on,
    once: function (ev, fn) {
      function w() { off(ev, w); fn.apply(null, arguments); }
      return on(ev, w);
    },
    off: off,
    removeListener: off,
    addListener: on,
    removeAllListeners: function (ev) {
      if (ev) delete listeners[ev]; else for (var k in listeners) delete listeners[k];
      return process;
    },
    emit: emit,
    listenerCount: function (ev) { return (listeners[ev] || []).length; },
    listeners: function (ev) { return (listeners[ev] || []).slice(); },
    eventNames: function () { return Object.keys(listeners); },
    setMaxListeners: function () { return process; },
    getMaxListeners: function () { return 10; },
    binding: function () { throw new Error("process.binding() is not supported in tynd-lite"); },
    dlopen: function () {
      throw new Error(
        "process.dlopen: native bindings (.node addons) are not supported on tynd-lite (QuickJS ABI != V8). " +
        "See https://tynd.kvnpetit.com/v1.0/guides/native-plugins-mapping for plugin alternatives."
      );
    },
    umask: function () { return 0; },
    getuid: function () { return 0; },
    geteuid: function () { return 0; },
    getgid: function () { return 0; },
    getegid: function () { return 0; },
    getgroups: function () { return []; },
    browser: false,
    stdout: makeStream("log"),
    stderr: makeStream("error"),
    stdin: { isTTY: false, on: function () { return this; }, resume: function () {}, pause: function () {}, read: function () { return null; } },
  };

  // process.nextTick aliases that npm packages reach for directly
  globalThis.queueMicrotask = globalThis.queueMicrotask || function (fn) { Promise.resolve().then(fn); };
  globalThis.setImmediate = globalThis.setImmediate || function (fn) {
    var args = Array.prototype.slice.call(arguments, 1);
    return setTimeout(function () { fn.apply(null, args); }, 0);
  };
  globalThis.clearImmediate = globalThis.clearImmediate || function (id) { clearTimeout(id); };

  globalThis.process = process;
  globalThis.global = globalThis;
  globalThis.__tynd_modules = globalThis.__tynd_modules || Object.create(null);
  globalThis.__tynd_modules["process"] = process;
  globalThis.__tynd_modules["node:process"] = process;
})();
