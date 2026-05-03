// Tynd-specific overrides on top of the unenv polyfills.
//
// unenv ships API-shape stubs (most fs/* methods throw "not implemented",
// the http client is partial, crypto.randomBytes returns Math.random in
// QuickJS contexts where Web Crypto isn't installed). We swap the slow /
// missing internals for native bridges (cap-std fs, ureq HTTP, blake3
// crypto) here — keeping unenv's API surface so npm packages still work.
//
// Order: runs AFTER the unenv bundle but BEFORE bun_shim / require shim,
// so `globalThis.__tynd_modules.fs.readFileSync` etc. are already populated
// by the time downstream code reaches them.
(function () {
  var modules = (globalThis.__tynd_modules = globalThis.__tynd_modules || Object.create(null));
  var Buffer = globalThis.Buffer;
  var b64Decode = globalThis.__tynd_b64_decode;
  var b64Encode = globalThis.__tynd_b64_encode;

  function need(name) {
    var fn = globalThis[name];
    if (typeof fn !== "function") {
      throw new Error("tynd_overrides: bridge '" + name + "' missing");
    }
    return fn;
  }

  // ---------------------------------------------------------------------
  // process.* — wire env / argv / pid / cwd via the Rust bridges. unenv's
  // stub returns hardcoded "browser" for everything; we patch in real OS
  // values from `__tynd_*` functions registered in `node_bridges.rs`.
  // ---------------------------------------------------------------------
  var process = modules["process"] || globalThis.process;
  if (process) {
    process.platform = (function () {
      try { return need("__tynd_platform__")(); } catch (_) { return "linux"; }
    })();
    process.arch = (function () {
      try { return need("__tynd_arch__")(); } catch (_) { return "x64"; }
    })();
    process.pid = (function () { try { return need("__tynd_pid__")(); } catch (_) { return 0; } })();
    process.argv = (function () { try { return need("__tynd_argv__")(); } catch (_) { return ["tynd"]; } })();
    process.execPath = process.argv[0] || "tynd";
    process.versions = Object.assign({}, process.versions || {}, {
      node: "20.0.0",
      tynd: "1.0.0",
    });
    process.version = "v20.0.0";
    process.cwd = function () { try { return need("__tynd_cwd__")(); } catch (_) { return "/"; } };
    process.chdir = function (p) { return need("__tynd_chdir__")(String(p)); };
    process.exit = function (code) { return need("__tynd_exit__")(code | 0); };
    process.hrtime = function (prev) {
      var ms = need("__tynd_hrtime_ms__")();
      var sec = Math.floor(ms / 1000);
      var nsec = Math.floor((ms - sec * 1000) * 1e6);
      if (prev) {
        var dSec = sec - prev[0];
        var dNsec = nsec - prev[1];
        if (dNsec < 0) { dSec -= 1; dNsec += 1e9; }
        return [dSec, dNsec];
      }
      return [sec, nsec];
    };
    process.hrtime.bigint = function () {
      try {
        var ms = need("__tynd_hrtime_ms__")();
        return BigInt(Math.floor(ms * 1e6));
      } catch (_) { return BigInt(0); }
    };
    process.uptime = function () {
      try { return need("__tynd_uptime_ms__")() / 1000; } catch (_) { return 0; }
    };

    // env — proxy reads / writes to native bridges so changes persist
    // across process.env access.
    var envCache = null;
    function loadEnv() {
      try {
        var keys = need("__tynd_env_keys__")();
        var out = Object.create(null);
        for (var i = 0; i < keys.length; i++) {
          var k = keys[i];
          var v = need("__tynd_env_get__")(k);
          if (v != null) out[k] = v;
        }
        envCache = out;
      } catch (_) { envCache = Object.create(null); }
      return envCache;
    }
    process.env = new Proxy(Object.create(null), {
      get: function (_t, k) {
        if (typeof k !== "string") return undefined;
        try { return need("__tynd_env_get__")(k); } catch (_) { return undefined; }
      },
      set: function (_t, k, v) {
        if (typeof k !== "string") return true;
        try { need("__tynd_env_set__")(k, String(v)); } catch (_) {}
        if (envCache) envCache[k] = String(v);
        return true;
      },
      deleteProperty: function (_t, k) {
        if (typeof k !== "string") return true;
        try { need("__tynd_env_del__")(k); } catch (_) {}
        if (envCache && k in envCache) delete envCache[k];
        return true;
      },
      has: function (_t, k) {
        if (typeof k !== "string") return false;
        try { return need("__tynd_env_get__")(k) != null; } catch (_) { return false; }
      },
      ownKeys: function () {
        try { return need("__tynd_env_keys__")(); } catch (_) { return []; }
      },
      getOwnPropertyDescriptor: function (_t, k) {
        try {
          var v = need("__tynd_env_get__")(String(k));
          if (v == null) return undefined;
          return { enumerable: true, configurable: true, writable: true, value: v };
        } catch (_) { return undefined; }
      },
    });
    // Re-expose process on globalThis (matches Node).
    globalThis.process = process;
  }

  // ---------------------------------------------------------------------
  // os.* — same idea: replace unenv's "" / 0 stubs with native probes.
  // ---------------------------------------------------------------------
  var os = modules["os"];
  if (os) {
    os.platform = function () { try { return need("__tynd_platform__")(); } catch (_) { return "linux"; } };
    os.arch = function () { try { return need("__tynd_arch__")(); } catch (_) { return "x64"; } };
    os.type = function () { try { return need("__tynd_os_type__")(); } catch (_) { return "Linux"; } };
    os.release = function () { try { return need("__tynd_os_release__")(); } catch (_) { return "0.0.0"; } };
    os.homedir = function () { try { return need("__tynd_os_homedir__")() || "/"; } catch (_) { return "/"; } };
    os.tmpdir = function () { try { return need("__tynd_os_tmpdir__")(); } catch (_) { return "/tmp"; } };
    os.hostname = function () { try { return need("__tynd_os_hostname__")() || "localhost"; } catch (_) { return "localhost"; } };
    os.totalmem = function () { try { return need("__tynd_os_totalmem__")(); } catch (_) { return 0; } };
    os.freemem = function () { try { return need("__tynd_os_freemem__")(); } catch (_) { return 0; } };
    os.uptime = function () { try { return need("__tynd_os_uptime__")(); } catch (_) { return 0; } };
    os.loadavg = function () { try { return need("__tynd_os_loadavg__")(); } catch (_) { return [0, 0, 0]; } };
    os.cpus = function () {
      var n = 1;
      try { n = need("__tynd_os_cpus__")(); } catch (_) {}
      var arr = [];
      for (var i = 0; i < n; i++) {
        arr.push({ model: "tynd-lite", speed: 0, times: { user: 0, nice: 0, sys: 0, idle: 0, irq: 0 } });
      }
      return arr;
    };
    os.endianness = function () {
      var b = new ArrayBuffer(2);
      new DataView(b).setInt16(0, 256, true);
      return new Int16Array(b)[0] === 256 ? "LE" : "BE";
    };
    os.networkInterfaces = function () { return {}; };
    os.userInfo = function () {
      var home = os.homedir();
      return { username: "tynd", uid: 0, gid: 0, shell: null, homedir: home };
    };
    os.EOL = (process && process.platform === "win32") ? "\r\n" : "\n";
    os.constants = os.constants || { signals: {}, errno: {} };
  }

  // ---------------------------------------------------------------------
  // fs — patch the actual sync/async methods to hit native cap-std
  // through `__tynd_fs_*`. unenv's _fs.cjs throws notImplemented for most
  // disk operations.
  // ---------------------------------------------------------------------
  function toBuf(b64) { return Buffer.from(b64Decode(b64)); }
  function fromBuf(buf) {
    return b64Encode(buf instanceof Uint8Array ? buf : new Uint8Array(buf));
  }
  function statResultWrap(r) {
    if (!r) return r;
    return Object.assign({}, r, {
      isFile: function () { return !!r.isFile; },
      isDirectory: function () { return !!r.isDirectory; },
      isSymbolicLink: function () { return !!r.isSymbolicLink; },
      isBlockDevice: function () { return false; },
      isCharacterDevice: function () { return false; },
      isFIFO: function () { return false; },
      isSocket: function () { return false; },
    });
  }

  var fsSync = {
    readFileSync: function (path, options) {
      var enc = typeof options === "string" ? options : (options && options.encoding) || null;
      if (enc) return need("__tynd_fs_read_text__")(String(path));
      return toBuf(need("__tynd_fs_read_bytes__")(String(path)));
    },
    writeFileSync: function (path, data, options) {
      var enc = typeof options === "string" ? options : (options && options.encoding) || null;
      if (typeof data === "string") {
        if (enc && enc !== "utf8" && enc !== "utf-8") {
          var buf = Buffer.from(data, enc);
          return need("__tynd_fs_write_bytes__")(String(path), fromBuf(buf));
        }
        return need("__tynd_fs_write_text__")(String(path), data);
      }
      var u8 = data instanceof Uint8Array
        ? data
        : (data && data.buffer
          ? new Uint8Array(data.buffer, data.byteOffset, data.byteLength)
          : new Uint8Array(data));
      return need("__tynd_fs_write_bytes__")(String(path), fromBuf(u8));
    },
    appendFileSync: function (path, data, options) {
      var existing;
      try { existing = fsSync.readFileSync(path); } catch (_) { existing = Buffer.alloc(0); }
      var add = typeof data === "string" ? Buffer.from(data, (options && options.encoding) || "utf8") : data;
      if (typeof existing === "string") existing = Buffer.from(existing, "utf8");
      return fsSync.writeFileSync(path, Buffer.concat([existing, add]));
    },
    existsSync: function (path) {
      try { return !!need("__tynd_fs_exists__")(String(path)); } catch (_) { return false; }
    },
    statSync: function (path) { return statResultWrap(need("__tynd_fs_stat__")(String(path))); },
    lstatSync: function (path) { return fsSync.statSync(path); },
    readdirSync: function (path, options) {
      var arr = need("__tynd_fs_readdir__")(String(path)) || [];
      var withFt = options && options.withFileTypes;
      if (withFt) {
        return arr.map(function (name) {
          return {
            name: name, parentPath: String(path),
            isFile: function () { try { return fsSync.statSync(String(path) + "/" + name).isFile(); } catch (_) { return false; } },
            isDirectory: function () { try { return fsSync.statSync(String(path) + "/" + name).isDirectory(); } catch (_) { return false; } },
            isSymbolicLink: function () { return false; },
          };
        });
      }
      return arr;
    },
    mkdirSync: function (path, options) {
      var rec = !!(options && (options.recursive || options === true));
      return need("__tynd_fs_mkdir__")(String(path), rec);
    },
    rmSync: function (path, options) {
      return need("__tynd_fs_rm__")(String(path), !!(options && (options.recursive || options.force)));
    },
    rmdirSync: function (path, options) {
      return need("__tynd_fs_rm__")(String(path), !!(options && options.recursive));
    },
    unlinkSync: function (path) { return need("__tynd_fs_rm__")(String(path), false); },
    renameSync: function (from, to) { return need("__tynd_fs_rename__")(String(from), String(to)); },
    copyFileSync: function (from, to) { return need("__tynd_fs_copy__")(String(from), String(to)); },
    realpathSync: function (path) { return need("__tynd_fs_realpath__")(String(path)); },
    chmodSync: function (path, mode) { return need("__tynd_fs_chmod__")(String(path), mode | 0); },
  };
  function asyncify(fn) {
    return function () {
      var args = arguments;
      return new Promise(function (resolve, reject) {
        try { resolve(fn.apply(null, args)); } catch (e) { reject(e); }
      });
    };
  }
  function callbackified(fn) {
    return function () {
      var args = Array.prototype.slice.call(arguments);
      var cb = typeof args[args.length - 1] === "function" ? args.pop() : null;
      try { var r = fn.apply(null, args); if (cb) cb(null, r); } catch (e) { if (cb) cb(e); }
    };
  }

  var fs = modules["fs"];
  if (fs) {
    Object.assign(fs, fsSync, {
      readFile: callbackified(fsSync.readFileSync),
      writeFile: callbackified(fsSync.writeFileSync),
      appendFile: callbackified(fsSync.appendFileSync),
      exists: function (p, cb) { try { cb && cb(fsSync.existsSync(p)); } catch (_) { cb && cb(false); } },
      stat: callbackified(fsSync.statSync), lstat: callbackified(fsSync.lstatSync),
      readdir: callbackified(fsSync.readdirSync), mkdir: callbackified(fsSync.mkdirSync),
      rm: callbackified(fsSync.rmSync), rmdir: callbackified(fsSync.rmdirSync),
      unlink: callbackified(fsSync.unlinkSync), rename: callbackified(fsSync.renameSync),
      copyFile: callbackified(fsSync.copyFileSync), realpath: callbackified(fsSync.realpathSync),
      chmod: callbackified(fsSync.chmodSync),
      access: function (p, _mode, cb) {
        if (typeof _mode === "function") { cb = _mode; }
        cb && (fsSync.existsSync(p) ? cb(null) : cb(new Error("ENOENT")));
      },
    });
  }
  var fsp = modules["fs/promises"];
  if (fsp) {
    Object.assign(fsp, {
      readFile: asyncify(fsSync.readFileSync), writeFile: asyncify(fsSync.writeFileSync),
      appendFile: asyncify(fsSync.appendFileSync),
      stat: asyncify(fsSync.statSync), lstat: asyncify(fsSync.lstatSync),
      readdir: asyncify(fsSync.readdirSync), mkdir: asyncify(fsSync.mkdirSync),
      rm: asyncify(fsSync.rmSync), rmdir: asyncify(fsSync.rmdirSync),
      unlink: asyncify(fsSync.unlinkSync),
      rename: asyncify(fsSync.renameSync), copyFile: asyncify(fsSync.copyFileSync),
      realpath: asyncify(fsSync.realpathSync), chmod: asyncify(fsSync.chmodSync),
      access: function (p) { return fsSync.existsSync(p) ? Promise.resolve() : Promise.reject(new Error("ENOENT: " + p)); },
    });
  }

  // ---------------------------------------------------------------------
  // http / https — unenv ships a stub. Replace with the same fetch-backed
  // ClientRequest the previous custom polyfill provided. Server APIs throw
  // a helpful "use os.http_server" error.
  // ---------------------------------------------------------------------
  var EE = (modules["events"] && (modules["events"].EventEmitter || modules["events"].default)) || globalThis.EventEmitter;
  if (!EE && modules["events"] && typeof modules["events"] === "function") EE = modules["events"];

  function ClientRequest(url, opts, cb) {
    EE.call(this);
    this._chunks = [];
    this._headers = Object.assign({}, (opts && opts.headers) || {});
    this._method = (opts && opts.method) || "GET";
    this._url = url;
    this._aborted = false;
    if (cb) this.once("response", cb);
  }
  ClientRequest.prototype = Object.create(EE.prototype);
  ClientRequest.prototype.write = function (chunk, _enc, cb) {
    if (typeof chunk === "string") chunk = Buffer.from(chunk, _enc || "utf8");
    this._chunks.push(chunk);
    if (cb) cb();
    return true;
  };
  ClientRequest.prototype.setHeader = function (k, v) { this._headers[k] = v; };
  ClientRequest.prototype.getHeader = function (k) { return this._headers[k]; };
  ClientRequest.prototype.removeHeader = function (k) { delete this._headers[k]; };
  ClientRequest.prototype.end = function (chunk, enc, cb) {
    if (typeof chunk === "function") { cb = chunk; chunk = null; }
    if (chunk != null) this.write(chunk, enc);
    var self = this;
    var body = self._chunks.length
      ? Buffer.concat(self._chunks.map(function (c) { return c instanceof Uint8Array ? c : Buffer.from(c); }))
      : undefined;
    var init = { method: self._method, headers: self._headers };
    if (body && self._method !== "GET" && self._method !== "HEAD") init.body = body;
    fetch(self._url, init).then(function (res) {
      var inc = new EE();
      inc.statusCode = res.status;
      inc.statusMessage = res.statusText;
      inc.headers = {};
      res.headers.forEach(function (v, k) { inc.headers[k.toLowerCase()] = v; });
      inc.httpVersion = "1.1";
      inc.setEncoding = function (e) { inc._encoding = e; };
      inc.pause = function () {};
      inc.resume = function () {};
      self.emit("response", inc);
      res.arrayBuffer().then(function (buf) {
        var b = Buffer.from(buf);
        var ck = inc._encoding ? b.toString(inc._encoding) : b;
        inc.emit("data", ck);
        inc.emit("end");
        inc.emit("close");
      }).catch(function (e) { inc.emit("error", e); });
      if (cb) cb();
    }).catch(function (e) { self.emit("error", e); if (cb) cb(e); });
    return self;
  };
  ClientRequest.prototype.abort = ClientRequest.prototype.destroy = function () {
    this._aborted = true; this.emit("close");
  };
  ClientRequest.prototype.setTimeout = function (_ms, fn) { if (fn) this.once("timeout", fn); return this; };

  function urlOf(input, opts) {
    if (typeof input === "string") return input;
    if (input && typeof input === "object" && input.host) {
      var proto = input.protocol || (opts && opts._proto) || "http:";
      var host = input.hostname || input.host;
      var port = input.port ? ":" + input.port : "";
      var path = input.path || "/";
      return proto + "//" + host + port + path;
    }
    return String(input);
  }
  function makeHttpMod(proto) {
    return {
      request: function (urlOrOpts, opts, cb) {
        if (typeof opts === "function") { cb = opts; opts = null; }
        if (typeof urlOrOpts === "object" && urlOrOpts !== null && !(urlOrOpts instanceof URL)) {
          opts = Object.assign({ _proto: proto }, urlOrOpts, opts);
          return new ClientRequest(urlOf(urlOrOpts, opts), opts, cb);
        }
        return new ClientRequest(urlOf(urlOrOpts, { _proto: proto }), opts || {}, cb);
      },
      get: function (urlOrOpts, opts, cb) {
        if (typeof opts === "function") { cb = opts; opts = null; }
        opts = Object.assign({ method: "GET" }, opts || {});
        var req = this.request(urlOrOpts, opts, cb);
        req.end();
        return req;
      },
      createServer: function () {
        throw new Error(
          "http.createServer is not supported in tynd-lite. " +
          "Use the host HTTP server: import { os } from '@tynd/core/client'; os.http_server.start(...). " +
          "See https://tynd.kvnpetit.com/v1.0/api/os/http",
        );
      },
      Agent: function Agent() {}, globalAgent: {},
      STATUS_CODES: { 200: "OK", 201: "Created", 204: "No Content", 301: "Moved Permanently", 302: "Found", 304: "Not Modified", 400: "Bad Request", 401: "Unauthorized", 403: "Forbidden", 404: "Not Found", 500: "Internal Server Error" },
      METHODS: ["GET", "POST", "PUT", "DELETE", "HEAD", "OPTIONS", "PATCH"],
      ClientRequest: ClientRequest,
      IncomingMessage: EE,
      ServerResponse: EE,
    };
  }
  modules["http"] = makeHttpMod("http:");
  modules["node:http"] = modules["http"];
  modules["https"] = makeHttpMod("https:");
  modules["node:https"] = modules["https"];

  // ---------------------------------------------------------------------
  // crypto — unenv has class shells but no real RNG / hash. Wire the
  // existing Web Crypto bridge (subtle is registered by `crypto.rs`) plus
  // sync sha helpers used by Node-style libs (e.g. `cookie-signature`,
  // `jsonwebtoken`).
  // ---------------------------------------------------------------------
  var nodeCrypto = modules["crypto"] || {};
  // Pull from globalThis.crypto (polyfilled by `crypto.js` / `crypto.rs`).
  var webCrypto = globalThis.crypto;

  nodeCrypto.randomBytes = function (size, cb) {
    var arr = new Uint8Array(size | 0);
    if (webCrypto && webCrypto.getRandomValues) webCrypto.getRandomValues(arr);
    else { for (var i = 0; i < arr.length; i++) arr[i] = (Math.random() * 256) | 0; }
    var buf = Buffer.from(arr);
    if (typeof cb === "function") { cb(null, buf); return; }
    return buf;
  };
  nodeCrypto.randomFillSync = function (buf, offset, size) {
    offset = offset | 0; size = size == null ? buf.length - offset : size | 0;
    var tmp = new Uint8Array(size);
    if (webCrypto && webCrypto.getRandomValues) webCrypto.getRandomValues(tmp);
    else { for (var i = 0; i < size; i++) tmp[i] = (Math.random() * 256) | 0; }
    for (var j = 0; j < size; j++) buf[offset + j] = tmp[j];
    return buf;
  };
  nodeCrypto.randomFill = function (buf, offset, size, cb) {
    if (typeof offset === "function") { cb = offset; offset = 0; size = buf.length; }
    else if (typeof size === "function") { cb = size; size = buf.length - (offset | 0); }
    try { var r = nodeCrypto.randomFillSync(buf, offset, size); cb && cb(null, r); }
    catch (e) { cb && cb(e); }
  };
  nodeCrypto.randomUUID = function () {
    if (webCrypto && webCrypto.randomUUID) return webCrypto.randomUUID();
    var b = new Uint8Array(16);
    nodeCrypto.randomFillSync(b, 0, 16);
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    var hex = Array.from(b).map(function (x) { return (x + 0x100).toString(16).slice(1); });
    return hex.slice(0, 4).join("") + "-" + hex.slice(4, 6).join("") + "-" + hex.slice(6, 8).join("") +
      "-" + hex.slice(8, 10).join("") + "-" + hex.slice(10, 16).join("");
  };
  nodeCrypto.randomInt = function (min, max, cb) {
    if (typeof min === "function") { cb = min; min = 0; max = 281474976710655; }
    if (typeof max === "function") { cb = max; max = min; min = 0; }
    var range = max - min;
    var v = (Math.random() * range) | 0;
    var r = v + min;
    if (cb) { cb(null, r); return; }
    return r;
  };
  nodeCrypto.timingSafeEqual = function (a, b) {
    if (a.length !== b.length) throw new RangeError("timingSafeEqual: input lengths must match");
    var diff = 0;
    for (var i = 0; i < a.length; i++) diff |= a[i] ^ b[i];
    return diff === 0;
  };

  // Hash via WebCrypto subtle (sha-1, sha-256, sha-384, sha-512). Sync
  // hash() requires polling so we expose async digest + a sync wrapper for
  // small inputs that does the digest on a TextEncoder + Web Crypto loop.
  // For heavy use (jwt, hmac), the userland `@noble/hashes` works.
  var hashAlgoMap = { sha1: "SHA-1", "sha-1": "SHA-1", sha256: "SHA-256", "sha-256": "SHA-256", sha384: "SHA-384", "sha-384": "SHA-384", sha512: "SHA-512", "sha-512": "SHA-512", md5: "MD5" };
  function Hash(algorithm) {
    if (!(this instanceof Hash)) return new Hash(algorithm);
    this._algo = String(algorithm).toLowerCase();
    this._chunks = [];
  }
  Hash.prototype.update = function (data, enc) {
    var chunk = typeof data === "string" ? Buffer.from(data, enc || "utf8") : (data instanceof Uint8Array ? data : Buffer.from(data));
    this._chunks.push(chunk);
    return this;
  };
  Hash.prototype.digest = function (enc) {
    var algo = hashAlgoMap[this._algo];
    if (!algo || algo === "MD5" || !webCrypto || !webCrypto.subtle) {
      throw new Error("crypto.createHash('" + this._algo + "'): unsupported in lite — use `@noble/hashes` for sync MD5 / non-Web-Crypto algorithms");
    }
    // Sync digest is impossible with Web Crypto; require .digest() callers
    // to use the async path. Throw a clear error.
    throw new Error("crypto.createHash('" + this._algo + "').digest() is sync but Web Crypto is async. Use crypto.subtle.digest('" + algo + "', data) directly, or @noble/hashes for sync.");
  };
  Hash.prototype.digestAsync = function (enc) {
    var algo = hashAlgoMap[this._algo];
    if (!algo || algo === "MD5" || !webCrypto || !webCrypto.subtle) {
      return Promise.reject(new Error("crypto.createHash('" + this._algo + "'): unsupported in lite"));
    }
    var data = Buffer.concat(this._chunks.map(function (c) { return c instanceof Uint8Array ? c : Buffer.from(c); }));
    return webCrypto.subtle.digest(algo, data).then(function (buf) {
      var b = Buffer.from(buf);
      return enc ? b.toString(enc) : b;
    });
  };
  nodeCrypto.createHash = function (algo) { return new Hash(algo); };
  nodeCrypto.getHashes = function () { return ["sha1", "sha256", "sha384", "sha512"]; };
  nodeCrypto.constants = nodeCrypto.constants || {};
  nodeCrypto.webcrypto = webCrypto;

  // Re-export so destructuring works.
  modules["crypto"] = nodeCrypto;
  modules["node:crypto"] = nodeCrypto;

  // ---------------------------------------------------------------------
  // Globals — expose EventEmitter on globalThis (some Node-style libs read
  // it without require()) and ensure querystring is at least a stub since
  // unenv doesn't ship one.
  // ---------------------------------------------------------------------
  if (EE && typeof globalThis.EventEmitter === "undefined") {
    globalThis.EventEmitter = EE;
  }
  // Patch unenv@1.10.0 events bug: emit() does
  // `(_listener.listener || _listener).apply(this, args)` which UNWRAPS
  // the once-wrapper before invoking it, so the `fired` flag never engages
  // and `removeListener` never runs. Fix: invoke the wrapper itself.
  if (EE && EE.prototype && EE.prototype.emit) {
    var origEmit = EE.prototype.emit;
    var origEmitTag = origEmit && origEmit.__tynd_patched__;
    if (!origEmitTag) {
      EE.prototype.emit = function (type) {
        var args = Array.prototype.slice.call(arguments, 1);
        if (type === "error") return origEmit.apply(this, arguments);
        var ev = this._events && this._events[type];
        if (!ev || ev.length === 0) return false;
        // Snapshot to allow listeners to remove themselves mid-iteration.
        var snap = ev.slice();
        for (var i = 0; i < snap.length; i++) {
          try { snap[i].apply(this, args); }
          catch (e) {
            if (typeof console !== "undefined" && console.error) {
              console.error("EventEmitter listener for '" + type + "' threw:", e);
            }
          }
        }
        return true;
      };
      EE.prototype.emit.__tynd_patched__ = true;
    }
  }

  // ---------------------------------------------------------------------
  // url.pathToFileURL / fileURLToPath — unenv just calls `new URL(path)`
  // which rejects bare absolute paths. Provide a real impl that prepends
  // `file://` and round-trips correctly on posix + win32.
  // ---------------------------------------------------------------------
  var urlMod = modules["url"];
  if (urlMod) {
    urlMod.pathToFileURL = function (p) {
      var s = String(p);
      if (s.charAt(0) === "/" || /^[A-Za-z]:[\\/]/.test(s)) {
        var posix = s.replace(/\\/g, "/");
        if (posix.charAt(0) !== "/") posix = "/" + posix;
        return new URL("file://" + posix);
      }
      return new URL(s);
    };
    urlMod.fileURLToPath = function (u) {
      var s = typeof u === "string" ? u : (u && u.href != null ? u.href : String(u));
      // file://host/path — extract the part after the host. We strip the
      // "file:" prefix and any leading "//" then the optional host segment.
      // file://host/path or file:///path. We always treat 3 slashes
      // (file:///x) as host="" so /x is the full path. With 2 slashes,
      // first segment is the host.
      var p;
      if (s.indexOf("file:///") === 0) {
        p = s.slice(7); // include the leading "/"
      } else if (s.indexOf("file://") === 0) {
        var rest = s.slice(7);
        var slash = rest.indexOf("/");
        p = slash < 0 ? "/" : rest.slice(slash);
      } else {
        p = null;
      }
      if (p != null) {
        try { p = decodeURIComponent(p); } catch (_) {}
        // Strip leading slash for win32 drive paths "/C:/..."
        if (/^\/[A-Za-z]:/.test(p)) p = p.slice(1);
        return p;
      }
      var u2 = typeof u === "string" ? new URL(u) : u;
      var path = decodeURIComponent(u2.pathname || "");
      if (/^\/[A-Za-z]:/.test(path)) path = path.slice(1);
      return path;
    };
    modules["url"] = urlMod;
    modules["node:url"] = urlMod;
  }

  // ---------------------------------------------------------------------
  // util.inspect — unenv's stub does `JSON.stringify` (quoted keys),
  // npm packages expect Node's unquoted `a: 1` style. Provide a small
  // Node-compatible inspect; same shape as the previous custom polyfill.
  // ---------------------------------------------------------------------
  var util = modules["util"];
  if (util) {
    function inspect(value, opts) {
      opts = opts || {};
      var depth = typeof opts === "number" ? opts : (opts.depth == null ? 2 : opts.depth);
      function fmt(v, d) {
        if (v === null) return "null";
        if (v === undefined) return "undefined";
        var t = typeof v;
        if (t === "string") return JSON.stringify(v);
        if (t === "number" || t === "boolean" || t === "bigint") return String(v);
        if (t === "function") return "[Function" + (v.name ? ": " + v.name : "") + "]";
        if (t === "symbol") return v.toString();
        if (d < 0) return Array.isArray(v) ? "[Array]" : "[Object]";
        if (Array.isArray(v)) {
          if (v.length === 0) return "[]";
          return "[ " + v.map(function (x) { return fmt(x, d - 1); }).join(", ") + " ]";
        }
        if (v instanceof Error) return v.stack || (v.name + ": " + v.message);
        if (v instanceof Date) return v.toISOString();
        if (v instanceof RegExp) return v.toString();
        var keys = Object.keys(v);
        if (keys.length === 0) return "{}";
        return "{ " + keys.map(function (k) {
          var safe = /^[A-Za-z_$][A-Za-z0-9_$]*$/.test(k) ? k : JSON.stringify(k);
          return safe + ": " + fmt(v[k], d - 1);
        }).join(", ") + " }";
      }
      return fmt(value, depth);
    }
    util.inspect = inspect;
    // Also expose util.format Node-style: %s/%d/%j/%o substitutions.
    util.format = function () {
      var args = Array.prototype.slice.call(arguments);
      var fmt = args.shift();
      if (typeof fmt !== "string") return [fmt].concat(args).map(function (a) { return inspect(a); }).join(" ");
      var i = 0;
      var out = fmt.replace(/%[sdjoO%]/g, function (m) {
        if (m === "%%") return "%";
        if (i >= args.length) return m;
        var a = args[i++];
        if (m === "%s") return String(a);
        if (m === "%d") return Number(a).toString();
        if (m === "%j") { try { return JSON.stringify(a); } catch (_) { return "[Circular]"; } }
        return inspect(a);
      });
      while (i < args.length) out += " " + (typeof args[i] === "string" ? args[i] : inspect(args[i++]));
      return out;
    };
    // util.callbackify — node-style; simplified
    util.callbackify = function (fn) {
      return function () {
        var args = Array.prototype.slice.call(arguments);
        var cb = args.pop();
        Promise.resolve().then(function () { return fn.apply(this, args); })
          .then(function (r) { cb(null, r); }, function (e) { cb(e); });
      };
    };
    // util.isDeepStrictEqual — needed by assert.deepStrictEqual
    if (!util.isDeepStrictEqual) {
      util.isDeepStrictEqual = function deepEq(a, b) {
        if (a === b) return true;
        if (typeof a !== typeof b) return false;
        if (a == null || b == null) return a === b;
        if (typeof a !== "object") return a === b;
        if (Array.isArray(a) !== Array.isArray(b)) return false;
        if (Array.isArray(a)) {
          if (a.length !== b.length) return false;
          for (var i = 0; i < a.length; i++) if (!deepEq(a[i], b[i])) return false;
          return true;
        }
        var ak = Object.keys(a), bk = Object.keys(b);
        if (ak.length !== bk.length) return false;
        for (var j = 0; j < ak.length; j++) {
          if (!Object.prototype.hasOwnProperty.call(b, ak[j])) return false;
          if (!deepEq(a[ak[j]], b[ak[j]])) return false;
        }
        return true;
      };
    }
    modules["util"] = util;
    modules["node:util"] = util;
  }

  // ---------------------------------------------------------------------
  // assert — unenv ships proxy mock; provide a real impl backed by
  // util.isDeepStrictEqual for libs that import it.
  // ---------------------------------------------------------------------
  function AssertionError(opts) {
    Error.call(this, opts.message);
    this.name = "AssertionError";
    this.message = opts.message;
    this.actual = opts.actual; this.expected = opts.expected; this.operator = opts.operator;
  }
  AssertionError.prototype = Object.create(Error.prototype);
  function assertFn(value, message) {
    if (!value) throw new AssertionError({ message: message || "false == true", actual: value, expected: true, operator: "==" });
  }
  assertFn.AssertionError = AssertionError;
  assertFn.ok = assertFn;
  assertFn.equal = function (a, e, m) { if (a != e) throw new AssertionError({ actual: a, expected: e, operator: "==", message: m || (a + " != " + e) }); };
  assertFn.notEqual = function (a, e, m) { if (a == e) throw new AssertionError({ actual: a, expected: e, operator: "!=", message: m || (a + " == " + e) }); };
  assertFn.strictEqual = function (a, e, m) { if (a !== e) throw new AssertionError({ actual: a, expected: e, operator: "===", message: m || (a + " !== " + e) }); };
  assertFn.notStrictEqual = function (a, e, m) { if (a === e) throw new AssertionError({ actual: a, expected: e, operator: "!==", message: m || (a + " === " + e) }); };
  var utilForAssert = modules["util"];
  assertFn.deepStrictEqual = function (a, e, m) {
    if (!(utilForAssert && utilForAssert.isDeepStrictEqual(a, e))) {
      throw new AssertionError({ actual: a, expected: e, operator: "deepStrictEqual", message: m || "values not deeply equal" });
    }
  };
  assertFn.deepEqual = assertFn.deepStrictEqual;
  assertFn.notDeepStrictEqual = function (a, e, m) {
    if (utilForAssert && utilForAssert.isDeepStrictEqual(a, e)) {
      throw new AssertionError({ actual: a, expected: e, operator: "notDeepStrictEqual", message: m || "values are deeply equal" });
    }
  };
  assertFn.notDeepEqual = assertFn.notDeepStrictEqual;
  assertFn.throws = function (fn, _expected, m) { try { fn(); } catch (_) { return; } throw new AssertionError({ message: m || "Missing expected exception" }); };
  assertFn.doesNotThrow = function (fn, m) { try { fn(); } catch (e) { throw new AssertionError({ message: m || ("Got unwanted exception: " + e) }); } };
  assertFn.rejects = function (p, _e, m) {
    return Promise.resolve(typeof p === "function" ? p() : p)
      .then(function () { throw new AssertionError({ message: m || "Missing expected rejection" }); }, function () {});
  };
  assertFn.doesNotReject = function (p, m) {
    return Promise.resolve(typeof p === "function" ? p() : p)
      .catch(function (e) { throw new AssertionError({ message: m || ("Got unwanted rejection: " + e) }); });
  };
  assertFn.fail = function (m) { throw new AssertionError({ message: m || "Failed" }); };
  assertFn.match = function (s, re, m) { if (!re.test(s)) throw new AssertionError({ message: m || (s + " does not match " + re) }); };
  assertFn.doesNotMatch = function (s, re, m) { if (re.test(s)) throw new AssertionError({ message: m || (s + " matches " + re) }); };
  assertFn.ifError = function (e) { if (e) throw e; };
  assertFn.strict = assertFn;
  modules["assert"] = assertFn;
  modules["node:assert"] = assertFn;
  modules["assert/strict"] = assertFn;
  modules["node:assert/strict"] = assertFn;

  // Always install — unenv ships a mock proxy here, we want the real impl.
  {
    var qs = {
      escape: function (s) { return encodeURIComponent(String(s)); },
      unescape: function (s) {
        try { return decodeURIComponent(String(s).replace(/\+/g, " ")); } catch (_) { return s; }
      },
      stringify: function (obj, sep, eq) {
        sep = sep || "&"; eq = eq || "=";
        if (obj == null || typeof obj !== "object") return "";
        var out = [];
        for (var k in obj) {
          var v = obj[k];
          var ek = encodeURIComponent(k);
          if (Array.isArray(v)) {
            for (var i = 0; i < v.length; i++) out.push(ek + eq + encodeURIComponent(v[i] == null ? "" : String(v[i])));
          } else {
            out.push(ek + eq + encodeURIComponent(v == null ? "" : String(v)));
          }
        }
        return out.join(sep);
      },
      parse: function (str, sep, eq) {
        sep = sep || "&"; eq = eq || "=";
        var out = Object.create(null);
        if (typeof str !== "string" || str.length === 0) return out;
        var parts = str.split(sep);
        for (var i = 0; i < parts.length; i++) {
          var idx = parts[i].indexOf(eq);
          var k, v;
          if (idx >= 0) { k = parts[i].slice(0, idx); v = parts[i].slice(idx + 1); }
          else { k = parts[i]; v = ""; }
          try { k = decodeURIComponent(k.replace(/\+/g, " ")); } catch (_) {}
          try { v = decodeURIComponent(v.replace(/\+/g, " ")); } catch (_) {}
          if (k in out) {
            if (Array.isArray(out[k])) out[k].push(v);
            else out[k] = [out[k], v];
          } else out[k] = v;
        }
        return out;
      },
      decode: null, encode: null,
    };
    qs.decode = qs.parse; qs.encode = qs.stringify;
    modules["querystring"] = qs;
    modules["node:querystring"] = qs;
  }
})();
