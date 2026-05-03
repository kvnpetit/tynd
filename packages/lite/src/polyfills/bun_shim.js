// Bun.* compat shim — covers the Bun-specific APIs that npm packages reach
// for at runtime (auth libs, DB libs, web frameworks). Goal: code written
// for Bun runs on tynd-lite without `if (typeof Bun !== 'undefined')` checks.
//
// Categories:
//   1. Aliases over standards (Bun.env, Bun.argv, Bun.password.hash) — full parity
//   2. fs-shimmed (Bun.file, Bun.write) — works via fs polyfill
//   3. Backed by host APIs (Bun.spawn / Bun.serve) — redirect to '@tynd/core/client'
//   4. Bun-internal (Bun.SQL, Bun.S3Client, Bun.dns) — throw with helpful redirect
(function () {
  var Buffer = globalThis.Buffer;
  var modules = globalThis.__tynd_modules || {};
  var fs = modules["fs"];
  var crypto = modules["crypto"];

  function unsupported(name, alt) {
    return function () {
      throw new Error("Bun." + name + " is not supported in tynd-lite. " + (alt || "") +
        " See https://tynd.kvnpetit.com/v1.0/guides/bun-compat for full Bun parity matrix.");
    };
  }

  function BunFile(path) {
    this._path = String(path);
  }
  BunFile.prototype.text = function () {
    return new Promise(function (res, rej) { try { res(fs.readFileSync(this._path, "utf8")); } catch (e) { rej(e); } }.bind(this));
  };
  BunFile.prototype.arrayBuffer = function () {
    return new Promise(function (res, rej) { try { var b = fs.readFileSync(this._path); res(b.buffer.slice(b.byteOffset, b.byteOffset + b.byteLength)); } catch (e) { rej(e); } }.bind(this));
  };
  BunFile.prototype.bytes = function () {
    return new Promise(function (res, rej) { try { res(new Uint8Array(fs.readFileSync(this._path))); } catch (e) { rej(e); } }.bind(this));
  };
  BunFile.prototype.json = function () {
    return this.text().then(JSON.parse);
  };
  BunFile.prototype.blob = function () {
    var self = this;
    return self.bytes().then(function (b) { return new Blob([b], { type: self.type }); });
  };
  BunFile.prototype.stream = function () {
    var self = this;
    return new ReadableStream({
      pull: function (ctrl) {
        try { ctrl.enqueue(new Uint8Array(fs.readFileSync(self._path))); ctrl.close(); }
        catch (e) { ctrl.error(e); }
      },
    });
  };
  Object.defineProperty(BunFile.prototype, "size", {
    get: function () { try { return fs.statSync(this._path).size; } catch (_) { return 0; } },
  });
  Object.defineProperty(BunFile.prototype, "type", {
    get: function () {
      var ext = this._path.toLowerCase();
      if (ext.endsWith(".json")) return "application/json";
      if (ext.endsWith(".html")) return "text/html";
      if (ext.endsWith(".js") || ext.endsWith(".mjs")) return "text/javascript";
      if (ext.endsWith(".css")) return "text/css";
      if (ext.endsWith(".png")) return "image/png";
      if (ext.endsWith(".jpg") || ext.endsWith(".jpeg")) return "image/jpeg";
      if (ext.endsWith(".txt") || ext.endsWith(".md")) return "text/plain";
      return "application/octet-stream";
    },
  });
  Object.defineProperty(BunFile.prototype, "name", { get: function () { return this._path.split(/[\\/]/).pop(); } });
  BunFile.prototype.exists = function () {
    var self = this;
    return new Promise(function (res) { try { res(fs.existsSync(self._path)); } catch (_) { res(false); } });
  };
  BunFile.prototype.writer = function () {
    var self = this;
    var chunks = [];
    return {
      write: function (chunk) {
        var b = typeof chunk === "string" ? Buffer.from(chunk, "utf8") : (chunk instanceof Uint8Array ? chunk : Buffer.from(chunk));
        chunks.push(b);
        return b.length;
      },
      flush: function () { return 0; },
      end: function () {
        try { fs.writeFileSync(self._path, Buffer.concat(chunks.map(function (c) { return c instanceof Uint8Array ? c : Buffer.from(c); }))); } catch (_) {}
        return 0;
      },
    };
  };

  function bunWrite(dest, data) {
    var path = dest instanceof BunFile ? dest._path : String(dest);
    var bytes;
    if (typeof data === "string") bytes = Buffer.from(data, "utf8");
    else if (data instanceof Uint8Array) bytes = data;
    else if (data instanceof ArrayBuffer) bytes = new Uint8Array(data);
    else if (data && typeof data.arrayBuffer === "function") {
      return data.arrayBuffer().then(function (ab) { fs.writeFileSync(path, new Uint8Array(ab)); return new Uint8Array(ab).length; });
    }
    else bytes = Buffer.from(String(data), "utf8");
    fs.writeFileSync(path, bytes);
    return Promise.resolve(bytes.length);
  }

  // Bun.password — alias over crypto.subtle digests + a salt scheme.
  // Format: `$tynd-lite-sha256$<salt-b64>$<digest-b64>`. Not bcrypt; for
  // legacy bcrypt hashes use `bcryptjs` (pure JS). Documented in the shim.
  var bunPassword = {
    hash: function (password, _opts) {
      var bytes = crypto.randomBytes(16);
      var salt = bytes.toString("base64");
      var combined = Buffer.concat([bytes, Buffer.from(String(password), "utf8")]);
      var digest = crypto.createHash("sha256").update(combined).digest("base64");
      return Promise.resolve("$tynd-lite-sha256$" + salt + "$" + digest);
    },
    hashSync: function (password) {
      var bytes = crypto.randomBytes(16);
      var salt = bytes.toString("base64");
      var combined = Buffer.concat([bytes, Buffer.from(String(password), "utf8")]);
      var digest = crypto.createHash("sha256").update(combined).digest("base64");
      return "$tynd-lite-sha256$" + salt + "$" + digest;
    },
    verify: function (password, hash) {
      var parts = String(hash).split("$");
      if (parts.length !== 4 || parts[1] !== "tynd-lite-sha256") {
        return Promise.reject(new Error("Bun.password.verify: tynd-lite only verifies its own scheme. For bcrypt/argon2 hashes use the 'bcryptjs' or '@noble/hashes' npm package."));
      }
      var salt = Buffer.from(parts[2], "base64");
      var combined = Buffer.concat([salt, Buffer.from(String(password), "utf8")]);
      var digest = crypto.createHash("sha256").update(combined).digest("base64");
      return Promise.resolve(digest === parts[3]);
    },
    verifySync: function (password, hash) {
      var parts = String(hash).split("$");
      if (parts.length !== 4 || parts[1] !== "tynd-lite-sha256") {
        throw new Error("Bun.password.verifySync: tynd-lite only verifies its own scheme.");
      }
      var salt = Buffer.from(parts[2], "base64");
      var combined = Buffer.concat([salt, Buffer.from(String(password), "utf8")]);
      var digest = crypto.createHash("sha256").update(combined).digest("base64");
      return digest === parts[3];
    },
  };

  // Bun.hash() short hashes — wyhash-flavoured fallback via SHA-256 truncated.
  function bunHashImpl(data) {
    var b = typeof data === "string" ? Buffer.from(data, "utf8") : Buffer.from(data);
    var d = crypto.createHash("sha256").update(b).digest();
    var n = BigInt(0);
    for (var i = 0; i < 8; i++) n = (n << BigInt(8)) | BigInt(d[i]);
    return n;
  }
  function bunHash(data, _seed) { return bunHashImpl(data); }
  bunHash.wyhash = bunHashImpl;
  bunHash.crc32 = function (data) {
    var b = typeof data === "string" ? Buffer.from(data, "utf8") : Buffer.from(data);
    var c = ~0; for (var i = 0; i < b.length; i++) {
      c = c ^ b[i];
      for (var j = 0; j < 8; j++) c = (c >>> 1) ^ (0xedb88320 & -(c & 1));
    }
    return (~c) >>> 0;
  };
  bunHash.adler32 = function (data) {
    var b = typeof data === "string" ? Buffer.from(data, "utf8") : Buffer.from(data);
    var a = 1, c = 0;
    for (var i = 0; i < b.length; i++) { a = (a + b[i]) % 65521; c = (c + a) % 65521; }
    return ((c << 16) | a) >>> 0;
  };

  // Bun.Glob — minimal subset using a regex compile.
  function Glob(pattern) { this._pattern = String(pattern); this._re = globToRegex(this._pattern); }
  Glob.prototype.match = function (path) { return this._re.test(String(path)); };
  Glob.prototype.scan = function (_opts) { throw new Error("Bun.Glob#scan requires a real fs walk; not implemented in lite. Use os.fs.scan from '@tynd/core/client'."); };
  Glob.prototype.scanSync = Glob.prototype.scan;
  function globToRegex(pat) {
    var re = "^";
    var i = 0;
    while (i < pat.length) {
      var c = pat.charAt(i);
      if (c === "*") {
        if (pat.charAt(i + 1) === "*") { re += ".*"; i += 2; }
        else { re += "[^/]*"; i++; }
      } else if (c === "?") { re += "[^/]"; i++; }
      else if (c === ".") { re += "\\."; i++; }
      else if ("()|+^$".indexOf(c) >= 0) { re += "\\" + c; i++; }
      else { re += c; i++; }
    }
    re += "$";
    return new RegExp(re);
  }

  var Bun = {
    env: globalThis.process ? process.env : {},
    argv: globalThis.process ? process.argv : [],
    main: globalThis.process ? process.execPath : "tynd-lite",
    version: "1.0.0-tynd-lite-shim",
    revision: "tynd-lite",
    file: function (path) { return new BunFile(path); },
    write: bunWrite,
    fileURLToPath: modules["url"] ? modules["url"].fileURLToPath : function (u) { return String(u).replace(/^file:\/\//, ""); },
    pathToFileURL: modules["url"] ? modules["url"].pathToFileURL : function (p) { return new URL("file://" + p); },
    sleep: function (ms) { return new Promise(function (r) { setTimeout(r, ms); }); },
    sleepSync: function (_ms) { /* QuickJS has no sync sleep — best-effort no-op */ },
    nanoseconds: function () { return BigInt(Math.round(performance.now() * 1e6)); },
    gc: function () { return 0; },
    indexOfLine: function (haystack, offset) {
      offset = offset | 0;
      var s = haystack instanceof Uint8Array ? Buffer.from(haystack).toString("utf8") : String(haystack);
      var idx = s.indexOf("\n", offset);
      return idx;
    },
    inspect: function (v) { return modules["util"] ? modules["util"].inspect(v) : String(v); },
    stringWidth: function (s) { return String(s || "").length; },
    deepEquals: function (a, b) { return modules["util"] && modules["util"].isDeepStrictEqual(a, b); },
    deepMatch: function (sub, sup) {
      if (sub === sup) return true;
      if (typeof sub !== "object" || sub === null) return false;
      if (typeof sup !== "object" || sup === null) return false;
      for (var k in sub) if (!Bun.deepMatch(sub[k], sup[k])) return false;
      return true;
    },
    escapeHTML: function (s) { return String(s).replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;").replace(/'/g, "&#39;"); },
    Glob: Glob,
    password: bunPassword,
    hash: bunHash,
    CryptoHasher: function (algo) { return crypto.createHash(algo); },
    BunFile: BunFile,
    readableStreamToText: function (s) { return new Response(s).text(); },
    readableStreamToJSON: function (s) { return new Response(s).json(); },
    readableStreamToArrayBuffer: function (s) { return new Response(s).arrayBuffer(); },
    readableStreamToBytes: function (s) { return new Response(s).arrayBuffer().then(function (b) { return new Uint8Array(b); }); },
    readableStreamToBlob: function (s) { return new Response(s).blob(); },
    readableStreamToFormData: function (s) { return new Response(s).formData(); },
    readableStreamToArray: function (s) {
      var out = []; var r = s.getReader();
      function step() { return r.read().then(function (v) { if (v.done) return out; out.push(v.value); return step(); }); }
      return step();
    },
    concatArrayBuffers: function (list) {
      var len = 0; for (var i = 0; i < list.length; i++) len += list[i].byteLength || list[i].length;
      var out = new Uint8Array(len); var pos = 0;
      for (var j = 0; j < list.length; j++) {
        var v = list[j] instanceof ArrayBuffer ? new Uint8Array(list[j]) : list[j];
        out.set(v, pos); pos += v.length;
      }
      return out.buffer;
    },
    pathToFileURL: function (p) { return modules["url"] ? modules["url"].pathToFileURL(p) : new URL("file://" + p); },
    resolveSync: function (id, _from) { return id; },
    resolve: function (id, _from) { return Promise.resolve(id); },

    // Bun.serve — redirect to host HTTP server
    serve: function (_opts) {
      throw new Error(
        "Bun.serve is not natively supported in tynd-lite. Tynd backends host their HTTP via the platform — use:\n" +
        "  import { os } from '@tynd/core/client';\n" +
        "  os.http_server.start({ port, handler })\n" +
        "See https://tynd.kvnpetit.com/v1.0/api/os/http_server"
      );
    },
    listen: function () { throw new Error("Bun.listen (raw TCP) unsupported in lite. Use WebSocket via the platform-standard API or os.tcp from '@tynd/core/client'."); },
    connect: function () { throw new Error("Bun.connect (raw TCP) unsupported in lite. Use fetch() or WebSocket."); },
    udpSocket: function () { throw new Error("Bun.udpSocket (UDP) unsupported in lite — UDP requires a native socket bridge."); },

    // Bun.spawn — redirect to host process API
    spawn: function (_cmd, _opts) {
      throw new Error(
        "Bun.spawn is not supported in tynd-lite. Use:\n" +
        "  import { os } from '@tynd/core/client';\n" +
        "  await os.process.spawn({ cmd: ['ls', '-la'], cwd: '...' })\n" +
        "Cross-platform process management with stdin/stdout/stderr streaming.\n" +
        "See https://tynd.kvnpetit.com/v1.0/api/os/process"
      );
    },
    spawnSync: function () { throw new Error("Bun.spawnSync unsupported in lite. Use os.process.spawn_sync from '@tynd/core/client'."); },
    which: function (_cmd) { return null; },

    // Bun-internal databases
    SQL: function () {
      throw new Error(
        "Bun.SQL is a Bun-internal Postgres client (Zig native). For tynd-lite use either:\n" +
        "  - @tynd/plugin-sql (cross-platform Rust native, drop-in API for SQLite/Postgres/MySQL)\n" +
        "  - the 'pg' npm package over WebSocket-bridged Postgres\n" +
        "See https://tynd.kvnpetit.com/v1.0/plugins/sql"
      );
    },
    sql: function () { throw new Error("Bun.sql unsupported in lite — use @tynd/plugin-sql."); },
    redis: function () { throw new Error("Bun.redis unsupported in lite — use 'ioredis' (pure-JS) or @tynd/plugin-redis."); },
    S3Client: function () {
      throw new Error("Bun.S3Client unsupported in lite — use the AWS SDK ('@aws-sdk/client-s3', pure-JS) or @tynd/plugin-s3.");
    },
    s3: { file: function () { throw new Error("Bun.s3 unsupported in lite — use @aws-sdk/client-s3 (pure-JS)."); } },

    // Bun.dns
    dns: {
      lookup: unsupported("dns.lookup", "Use IP literals (fetch supports them) or resolve via your hostings DNS layer."),
      resolve: unsupported("dns.resolve", "Use a DNS-over-HTTPS query against fetch() or a pure-JS DNS lib."),
    },

    // Bun.PeekStatus/peek/etc — minor utility helpers
    peek: function (p) { return p; },
    plugin: function () { throw new Error("Bun.plugin (bundler plugins) unsupported at runtime. Define plugins in tynd.config.ts and rebuild."); },
    semver: {
      satisfies: function (v, _r) { return false; }, // best-effort
      order: function (a, b) { return String(a).localeCompare(String(b)); },
    },
  };

  // Aliases for everything Bun ships.
  Bun.allocUnsafe = function (n) { return new Uint8Array(n); };
  Bun.openInEditor = unsupported("openInEditor", "Use os.shell.open from '@tynd/core/client'.");
  Bun.fs = function () { return modules["fs"]; };
  Bun.MD5 = function () { throw new Error("Bun.MD5 unsupported in lite — use @noble/hashes md5."); };
  Bun.SHA1 = function () { throw new Error("Bun.SHA1 unsupported in lite — use @noble/hashes sha1."); };
  Bun.SHA256 = function () { return crypto.createHash("sha256"); };
  Bun.SHA384 = function () { return crypto.createHash("sha384"); };
  Bun.SHA512 = function () { return crypto.createHash("sha512"); };
  Bun.SHA512_256 = function () { return crypto.createHash("sha512"); };

  globalThis.Bun = Bun;
})();
