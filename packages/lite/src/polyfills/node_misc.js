// Misc Node modules: assert, child_process, dns, net, tls, worker_threads,
// perf_hooks, timers, string_decoder, url (Node-style), readline, zlib,
// constants, vm, console (node-style addon), inspector.
//
// Most are either thin wrappers over what's already polyfilled (web URL,
// EventEmitter, perf.now, console) or "throw helpful unsupported error".
(function () {
  var EE = globalThis.EventEmitter;
  var Buffer = globalThis.Buffer;
  var modules = (globalThis.__tynd_modules = globalThis.__tynd_modules || Object.create(null));

  // ---- assert ----------------------------------------------------------
  function AssertionError(opts) {
    Error.call(this, opts.message);
    this.name = "AssertionError";
    this.message = opts.message;
    this.actual = opts.actual; this.expected = opts.expected; this.operator = opts.operator;
  }
  AssertionError.prototype = Object.create(Error.prototype);
  function assert(value, message) { if (!value) throw new AssertionError({ message: message || "false == true", actual: value, expected: true, operator: "==" }); }
  assert.AssertionError = AssertionError;
  assert.ok = assert;
  assert.equal = function (a, e, m) { if (a != e) throw new AssertionError({ actual: a, expected: e, operator: "==", message: m || (a + " != " + e) }); };
  assert.notEqual = function (a, e, m) { if (a == e) throw new AssertionError({ actual: a, expected: e, operator: "!=", message: m || (a + " == " + e) }); };
  assert.strictEqual = function (a, e, m) { if (a !== e) throw new AssertionError({ actual: a, expected: e, operator: "===", message: m || (a + " !== " + e) }); };
  assert.notStrictEqual = function (a, e, m) { if (a === e) throw new AssertionError({ actual: a, expected: e, operator: "!==", message: m || (a + " === " + e) }); };
  var util = modules["util"];
  assert.deepStrictEqual = function (a, e, m) { if (!(util && util.isDeepStrictEqual(a, e))) throw new AssertionError({ actual: a, expected: e, operator: "deepStrictEqual", message: m || "values not deeply equal" }); };
  assert.deepEqual = assert.deepStrictEqual;
  assert.notDeepStrictEqual = function (a, e, m) { if (util && util.isDeepStrictEqual(a, e)) throw new AssertionError({ actual: a, expected: e, operator: "notDeepStrictEqual", message: m || "values are deeply equal" }); };
  assert.notDeepEqual = assert.notDeepStrictEqual;
  assert.throws = function (fn, _expected, m) { try { fn(); } catch (_) { return; } throw new AssertionError({ message: m || "Missing expected exception" }); };
  assert.doesNotThrow = function (fn, m) { try { fn(); } catch (e) { throw new AssertionError({ message: m || ("Got unwanted exception: " + e) }); } };
  assert.rejects = function (p, _e, m) { return Promise.resolve(typeof p === "function" ? p() : p).then(function () { throw new AssertionError({ message: m || "Missing expected rejection" }); }, function () {}); };
  assert.doesNotReject = function (p, m) { return Promise.resolve(typeof p === "function" ? p() : p).catch(function (e) { throw new AssertionError({ message: m || ("Got unwanted rejection: " + e) }); }); };
  assert.fail = function (m) { throw new AssertionError({ message: m || "Failed" }); };
  assert.match = function (s, re, m) { if (!re.test(s)) throw new AssertionError({ message: m || (s + " does not match " + re) }); };
  assert.doesNotMatch = function (s, re, m) { if (re.test(s)) throw new AssertionError({ message: m || (s + " matches " + re) }); };
  assert.ifError = function (e) { if (e) throw e; };
  assert.strict = assert;
  modules["assert"] = assert; modules["node:assert"] = assert;
  modules["assert/strict"] = assert; modules["node:assert/strict"] = assert;

  // ---- child_process ---------------------------------------------------
  var cp = {
    spawn: function () { throw new Error("child_process.spawn is not supported in tynd-lite. Use os.process.spawn from '@tynd/core/client' (real cross-platform process management). See https://tynd.kvnpetit.com/v1.0/api/os/process"); },
    exec: function (_cmd, _opts, cb) {
      var fn = typeof _opts === "function" ? _opts : cb;
      if (fn) fn(new Error("child_process.exec is not supported in tynd-lite. Use os.process.exec from '@tynd/core/client'."));
    },
    execSync: function () { throw new Error("child_process.execSync is not supported in tynd-lite. Use os.process.exec_sync from '@tynd/core/client'."); },
    execFile: function (_f, _a, _o, cb) { var fn = typeof _o === "function" ? _o : (typeof _a === "function" ? _a : cb); if (fn) fn(new Error("child_process.execFile is not supported in tynd-lite. Use os.process.")); },
    execFileSync: function () { throw new Error("child_process.execFileSync is not supported in tynd-lite. Use os.process."); },
    fork: function () { throw new Error("child_process.fork is not supported in tynd-lite. Use Workers API (worker_threads polyfill is also unsupported)."); },
    spawnSync: function () { throw new Error("child_process.spawnSync is not supported in tynd-lite. Use os.process from '@tynd/core/client'."); },
  };
  modules["child_process"] = cp; modules["node:child_process"] = cp;

  // ---- dns -------------------------------------------------------------
  var dns = {
    lookup: function (_h, _o, cb) { var fn = typeof _o === "function" ? _o : cb; fn && fn(new Error("dns.lookup unsupported in tynd-lite. Use fetch/IP literal or os.network from '@tynd/core/client'.")); },
    resolve: function (_h, _r, cb) { var fn = typeof _r === "function" ? _r : cb; fn && fn(new Error("dns.resolve unsupported in tynd-lite.")); },
    promises: { lookup: function () { return Promise.reject(new Error("dns.lookup unsupported in tynd-lite.")); }, resolve: function () { return Promise.reject(new Error("dns.resolve unsupported in tynd-lite.")); } },
  };
  modules["dns"] = dns; modules["node:dns"] = dns;
  modules["dns/promises"] = dns.promises; modules["node:dns/promises"] = dns.promises;

  // ---- net / tls -------------------------------------------------------
  function netUnsupported(name) {
    return function () {
      throw new Error("net." + name + " (raw TCP sockets) is not supported in tynd-lite. Use the WebSocket API for full-duplex connections, or os.tcp from '@tynd/core/client' if available.");
    };
  }
  var net = {
    createServer: netUnsupported("createServer"), createConnection: netUnsupported("createConnection"),
    connect: netUnsupported("connect"), Socket: function () { throw new Error("net.Socket unsupported in tynd-lite."); },
    isIP: function (s) { if (/^(\d{1,3}\.){3}\d{1,3}$/.test(s)) return 4; if (/:/.test(s)) return 6; return 0; },
    isIPv4: function (s) { return /^(\d{1,3}\.){3}\d{1,3}$/.test(s); },
    isIPv6: function (s) { return /:/.test(s); },
  };
  modules["net"] = net; modules["node:net"] = net;
  var tls = Object.assign({}, net, { TLSSocket: function () { throw new Error("tls.TLSSocket unsupported in tynd-lite."); } });
  modules["tls"] = tls; modules["node:tls"] = tls;

  // ---- dgram (UDP) -----------------------------------------------------
  var dgram = {
    createSocket: function () { throw new Error("dgram.createSocket (UDP) is not supported in tynd-lite. UDP requires a native socket bridge — file an issue if you need it."); },
  };
  modules["dgram"] = dgram; modules["node:dgram"] = dgram;

  // ---- worker_threads --------------------------------------------------
  var workerThreads = {
    Worker: function () { throw new Error("worker_threads.Worker is not supported in tynd-lite (single-threaded QuickJS). Use os.workers from '@tynd/core/client' for off-main-thread compute, or split into separate tynd backends."); },
    isMainThread: true, parentPort: null, workerData: null, threadId: 0,
    MessageChannel: function () { throw new Error("MessageChannel unsupported in tynd-lite."); },
    BroadcastChannel: globalThis.BroadcastChannel,
  };
  modules["worker_threads"] = workerThreads; modules["node:worker_threads"] = workerThreads;

  // ---- perf_hooks ------------------------------------------------------
  var perfHooks = {
    performance: globalThis.performance || { now: function () { return Date.now(); } },
    PerformanceObserver: function () { return { observe: function () {}, disconnect: function () {} }; },
    monitorEventLoopDelay: function () { return { enable: function () {}, disable: function () {}, percentile: function () { return 0; } }; },
  };
  modules["perf_hooks"] = perfHooks; modules["node:perf_hooks"] = perfHooks;

  // ---- timers ----------------------------------------------------------
  var timers = {
    setTimeout: globalThis.setTimeout, clearTimeout: globalThis.clearTimeout,
    setInterval: globalThis.setInterval, clearInterval: globalThis.clearInterval,
    setImmediate: globalThis.setImmediate, clearImmediate: globalThis.clearImmediate,
    promises: {
      setTimeout: function (ms, value) { return new Promise(function (r) { setTimeout(function () { r(value); }, ms); }); },
      setImmediate: function (value) { return new Promise(function (r) { setTimeout(function () { r(value); }, 0); }); },
      setInterval: function () { throw new Error("timers/promises.setInterval unsupported (no async iterator wiring)"); },
    },
  };
  modules["timers"] = timers; modules["node:timers"] = timers;
  modules["timers/promises"] = timers.promises; modules["node:timers/promises"] = timers.promises;

  // ---- string_decoder --------------------------------------------------
  function StringDecoder(encoding) {
    this.encoding = (encoding || "utf8").toLowerCase();
    this._td = new TextDecoder(this.encoding === "utf8" || this.encoding === "utf-8" ? "utf-8" : this.encoding);
  }
  StringDecoder.prototype.write = function (b) { return this._td.decode(b instanceof Uint8Array ? b : Buffer.from(b), { stream: true }); };
  StringDecoder.prototype.end = function (b) { return b ? this._td.decode(b instanceof Uint8Array ? b : Buffer.from(b)) : this._td.decode(); };
  modules["string_decoder"] = { StringDecoder: StringDecoder };
  modules["node:string_decoder"] = modules["string_decoder"];

  // ---- url (Node-style alias on top of WHATWG URL) ---------------------
  var nodeUrl = {
    URL: globalThis.URL,
    URLSearchParams: globalThis.URLSearchParams,
    parse: function (str, parseQueryString, _slashesDenoteHost) {
      var u = new URL(str, "http://_/");
      var query = parseQueryString ? Object.fromEntries(u.searchParams.entries()) : (u.search ? u.search.slice(1) : null);
      return {
        href: u.href, protocol: u.protocol, slashes: true,
        host: u.host, hostname: u.hostname, port: u.port,
        pathname: u.pathname, search: u.search, query: query,
        hash: u.hash, path: u.pathname + u.search,
      };
    },
    format: function (u) {
      if (typeof u === "string") return u;
      if (u instanceof URL) return u.toString();
      var s = "";
      if (u.protocol) s += u.protocol + (u.slashes !== false ? "//" : "");
      if (u.host) s += u.host;
      else if (u.hostname) s += u.hostname + (u.port ? ":" + u.port : "");
      if (u.pathname) s += u.pathname;
      if (u.search) s += u.search;
      else if (u.query) s += "?" + (typeof u.query === "string" ? u.query : new URLSearchParams(u.query).toString());
      if (u.hash) s += u.hash;
      return s;
    },
    resolve: function (from, to) { try { return new URL(to, from).toString(); } catch (_) { return to; } },
    fileURLToPath: function (u) {
      var s = u instanceof URL ? u.href : String(u);
      if (!s.startsWith("file:")) throw new TypeError("Not a file:// URL");
      var path = s.slice(s.indexOf("://") + 3);
      var hashIdx = path.indexOf("#"); if (hashIdx >= 0) path = path.slice(0, hashIdx);
      var qIdx = path.indexOf("?"); if (qIdx >= 0) path = path.slice(0, qIdx);
      // Strip a leading host (file://localhost/...).
      var slashIdx = path.indexOf("/"); if (slashIdx > 0) path = path.slice(slashIdx);
      try { path = decodeURIComponent(path); } catch (_) {}
      // Windows: file:///C:/foo => C:/foo
      if (/^\/[A-Za-z]:/.test(path)) path = path.slice(1);
      return path;
    },
    pathToFileURL: function (p) {
      var s = String(p);
      if (/^[A-Za-z]:[\\/]/.test(s)) s = "/" + s.replace(/\\/g, "/");
      return new URL("file://" + s.split("/").map(function (seg) { return encodeURIComponent(seg); }).join("/"));
    },
    domainToASCII: function (d) { return String(d); },
    domainToUnicode: function (d) { return String(d); },
  };
  modules["url"] = nodeUrl; modules["node:url"] = nodeUrl;

  // ---- readline (minimal) ---------------------------------------------
  var rl = {
    createInterface: function () {
      var iface = new EE();
      iface.close = function () { this.emit("close"); };
      iface.question = function (_q, cb) { cb && cb(""); };
      iface.write = function () {};
      iface.pause = iface.resume = iface.setPrompt = iface.prompt = function () {};
      return iface;
    },
    emitKeypressEvents: function () {},
    cursorTo: function () { return true; }, clearLine: function () { return true; }, clearScreenDown: function () { return true; },
    moveCursor: function () { return true; },
  };
  modules["readline"] = rl; modules["node:readline"] = rl;

  // ---- zlib (delegate via plugin recommendation) -----------------------
  function zlibUnsupported(name) {
    return function () { throw new Error("zlib." + name + " is not supported in tynd-lite. Use the Web 'CompressionStream'/'DecompressionStream' (gzip/deflate/br) or the pure-JS 'pako' package."); };
  }
  var zlib = {
    createGzip: zlibUnsupported("createGzip"), createGunzip: zlibUnsupported("createGunzip"),
    createDeflate: zlibUnsupported("createDeflate"), createInflate: zlibUnsupported("createInflate"),
    createBrotliCompress: zlibUnsupported("createBrotliCompress"), createBrotliDecompress: zlibUnsupported("createBrotliDecompress"),
    gzip: function (_b, cb) { cb && cb(new Error("zlib.gzip unsupported in lite — use CompressionStream or pako.")); },
    gunzip: function (_b, cb) { cb && cb(new Error("zlib.gunzip unsupported in lite — use DecompressionStream or pako.")); },
    deflate: function (_b, cb) { cb && cb(new Error("zlib.deflate unsupported in lite — use pako.")); },
    inflate: function (_b, cb) { cb && cb(new Error("zlib.inflate unsupported in lite — use pako.")); },
    constants: { Z_BEST_COMPRESSION: 9, Z_DEFAULT_COMPRESSION: -1, Z_FILTERED: 1, Z_HUFFMAN_ONLY: 2, Z_RLE: 3, Z_FIXED: 4, Z_DEFAULT_STRATEGY: 0 },
  };
  modules["zlib"] = zlib; modules["node:zlib"] = zlib;

  // ---- vm (no-op stubs that defer to globalThis-eval) ------------------
  var vm = {
    runInThisContext: function (code) { return (0, eval)(String(code)); },
    runInNewContext: function (code, ctx) { var keys = ctx ? Object.keys(ctx) : []; var vals = keys.map(function (k) { return ctx[k]; }); return Function.apply(null, keys.concat(["return (" + code + ")"])).apply(null, vals); },
    createContext: function (sandbox) { return sandbox || {}; },
    isContext: function () { return true; },
    Script: function (code) { this.code = String(code); this.runInThisContext = function () { return (0, eval)(this.code); }; this.runInNewContext = function (ctx) { return vm.runInNewContext(this.code, ctx); }; },
  };
  modules["vm"] = vm; modules["node:vm"] = vm;

  // ---- module / inspector / domain stubs -------------------------------
  modules["inspector"] = { open: function () {}, close: function () {}, url: function () { return null; } };
  modules["node:inspector"] = modules["inspector"];
  modules["domain"] = { create: function () { var e = new EE(); e.run = function (fn) { try { fn(); } catch (err) { e.emit("error", err); } }; e.add = function () {}; e.bind = function (fn) { return fn; }; return e; } };
  modules["node:domain"] = modules["domain"];
  modules["punycode"] = { encode: function (s) { return s; }, decode: function (s) { return s; }, toASCII: function (s) { return s; }, toUnicode: function (s) { return s; }, ucs2: { encode: function (a) { return String.fromCharCode.apply(null, a); }, decode: function (s) { var r = []; for (var i = 0; i < s.length; i++) r.push(s.charCodeAt(i)); return r; } } };
  modules["node:punycode"] = modules["punycode"];
  modules["console"] = globalThis.console;
  modules["node:console"] = globalThis.console;
  modules["constants"] = { O_RDONLY: 0, O_WRONLY: 1, O_RDWR: 2 };
  modules["node:constants"] = modules["constants"];
  modules["module"] = {
    Module: function () {},
    builtinModules: ["assert", "buffer", "child_process", "console", "constants", "crypto", "dns", "events", "fs", "http", "https", "net", "os", "path", "perf_hooks", "process", "punycode", "querystring", "readline", "stream", "string_decoder", "timers", "tls", "url", "util", "vm", "worker_threads", "zlib"],
    createRequire: function (_url) { return globalThis.require; },
  };
  modules["node:module"] = modules["module"];
  // node:test (minimal stub)
  function _test(name, fn) { try { Promise.resolve(typeof fn === "function" ? fn({ name: name }) : null).catch(function (e) { console.error("test '" + name + "' failed:", e); }); } catch (e) { console.error("test '" + name + "' failed:", e); } }
  _test.describe = function (n, fn) { console.log("describe:", n); fn && fn(); };
  _test.it = _test; _test.test = _test;
  _test.skip = function () {}; _test.todo = function () {};
  _test.before = function (fn) { fn && fn(); }; _test.after = function (fn) { fn && fn(); };
  _test.beforeEach = function () {}; _test.afterEach = function () {};
  modules["node:test"] = _test; modules["test"] = _test;
})();
