// node:fs / fs/promises — sync + async file APIs backed by host bridges.
//
// Bridges (see polyfills::process_rs::register_fs):
//   __tynd_fs_read_text__(path)             -> string  (throws on err)
//   __tynd_fs_read_bytes__(path)            -> base64 string
//   __tynd_fs_write_text__(path, contents)  -> void
//   __tynd_fs_write_bytes__(path, b64)      -> void
//   __tynd_fs_exists__(path)                -> bool
//   __tynd_fs_stat__(path)                  -> { size, isFile, isDirectory, mtimeMs, ... }
//   __tynd_fs_readdir__(path)               -> string[]
//   __tynd_fs_mkdir__(path, recursive)      -> void
//   __tynd_fs_rm__(path, recursive)         -> void
//   __tynd_fs_rename__(from, to)            -> void
//   __tynd_fs_copy__(from, to)              -> void
//   __tynd_fs_realpath__(path)              -> string
//   __tynd_fs_chmod__(path, mode)           -> void
//
// Streams: createReadStream / createWriteStream return a node-stream-style
// Readable / Writable backed by sync read/write (chunked for large files).
(function () {
  var b64Decode = globalThis.__tynd_b64_decode;
  var b64Encode = globalThis.__tynd_b64_encode;
  var Buffer = globalThis.Buffer;
  var streamMod = globalThis.__tynd_modules && globalThis.__tynd_modules["stream"];
  var Readable = streamMod && streamMod.Readable;
  var Writable = streamMod && streamMod.Writable;

  function need(name) {
    var fn = globalThis[name];
    if (typeof fn !== "function") throw new Error("fs: bridge '" + name + "' missing — call site lacks the host bridge wired in");
    return fn;
  }

  function toBuf(b64) { return Buffer.from(b64Decode(b64)); }
  function fromBuf(buf) { return b64Encode(buf instanceof Uint8Array ? buf : new Uint8Array(buf)); }

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

  // --- sync ---------------------------------------------------------------
  function readFileSync(path, options) {
    var enc = typeof options === "string" ? options : (options && options.encoding) || null;
    if (enc) {
      var s = need("__tynd_fs_read_text__")(String(path));
      return s;
    }
    var b = need("__tynd_fs_read_bytes__")(String(path));
    return toBuf(b);
  }
  function writeFileSync(path, data, options) {
    var enc = typeof options === "string" ? options : (options && options.encoding) || null;
    if (typeof data === "string") {
      if (enc && enc !== "utf8" && enc !== "utf-8") {
        var buf = Buffer.from(data, enc);
        return need("__tynd_fs_write_bytes__")(String(path), fromBuf(buf));
      }
      return need("__tynd_fs_write_text__")(String(path), data);
    }
    var u8 = data instanceof Uint8Array ? data : (data && data.buffer ? new Uint8Array(data.buffer, data.byteOffset, data.byteLength) : new Uint8Array(data));
    return need("__tynd_fs_write_bytes__")(String(path), fromBuf(u8));
  }
  function appendFileSync(path, data, options) {
    var existing;
    try { existing = readFileSync(path); } catch (_) { existing = Buffer.alloc(0); }
    var add = typeof data === "string" ? Buffer.from(data, (options && options.encoding) || "utf8") : data;
    if (typeof existing === "string") existing = Buffer.from(existing, "utf8");
    return writeFileSync(path, Buffer.concat([existing, add]));
  }
  function existsSync(path) { try { return !!need("__tynd_fs_exists__")(String(path)); } catch (_) { return false; } }
  function statSync(path) { return statResultWrap(need("__tynd_fs_stat__")(String(path))); }
  function lstatSync(path) { return statSync(path); } // no symlink distinction in lite
  function readdirSync(path, options) {
    var arr = need("__tynd_fs_readdir__")(String(path)) || [];
    var withFt = options && options.withFileTypes;
    if (withFt) {
      return arr.map(function (name) {
        return {
          name: name, parentPath: String(path),
          isFile: function () { try { return statSync(String(path) + "/" + name).isFile(); } catch (_) { return false; } },
          isDirectory: function () { try { return statSync(String(path) + "/" + name).isDirectory(); } catch (_) { return false; } },
          isSymbolicLink: function () { return false; },
        };
      });
    }
    return arr;
  }
  function mkdirSync(path, options) {
    var rec = !!(options && (options.recursive || options === true));
    return need("__tynd_fs_mkdir__")(String(path), rec);
  }
  function rmSync(path, options) {
    return need("__tynd_fs_rm__")(String(path), !!(options && (options.recursive || options.force)));
  }
  var rmdirSync = rmSync;
  function unlinkSync(path) { return need("__tynd_fs_rm__")(String(path), false); }
  function renameSync(from, to) { return need("__tynd_fs_rename__")(String(from), String(to)); }
  function copyFileSync(from, to) { return need("__tynd_fs_copy__")(String(from), String(to)); }
  function realpathSync(path) { return need("__tynd_fs_realpath__")(String(path)); }
  function chmodSync(path, mode) { return need("__tynd_fs_chmod__")(String(path), mode | 0); }

  function asyncify(fn) { return function () { var args = arguments; return new Promise(function (resolve, reject) { try { resolve(fn.apply(null, args)); } catch (e) { reject(e); } }); }; }
  function callbackified(fn) {
    return function () {
      var args = Array.prototype.slice.call(arguments);
      var cb = typeof args[args.length - 1] === "function" ? args.pop() : null;
      try { var r = fn.apply(null, args); if (cb) cb(null, r); }
      catch (e) { if (cb) cb(e); }
    };
  }

  function createReadStream(path, opts) {
    if (!Readable) throw new Error("fs.createReadStream requires stream polyfill");
    opts = opts || {};
    var data;
    try { data = readFileSync(path); } catch (e) { var s = new Readable(); Promise.resolve().then(function () { s.emit("error", e); }); return s; }
    var enc = opts.encoding || null;
    var stream = new Readable();
    Promise.resolve().then(function () {
      stream.emit("open");
      var chunk = enc ? (typeof data === "string" ? data : data.toString(enc)) : data;
      stream.emit("data", chunk);
      stream.emit("end");
      stream.emit("close");
    });
    return stream;
  }
  function createWriteStream(path, opts) {
    if (!Writable) throw new Error("fs.createWriteStream requires stream polyfill");
    opts = opts || {};
    var chunks = [];
    var stream = new Writable({
      write: function (chunk, _enc, cb) {
        chunks.push(typeof chunk === "string" ? Buffer.from(chunk, opts.encoding || "utf8") : chunk);
        cb && cb();
      },
    });
    var origEnd = stream.end;
    stream.end = function (chunk, enc, cb) {
      if (chunk != null && typeof chunk !== "function") this.write(chunk, enc);
      try { writeFileSync(path, Buffer.concat(chunks.map(function (c) { return c instanceof Uint8Array ? c : Buffer.from(c); }))); }
      catch (e) { var self = this; Promise.resolve().then(function () { self.emit("error", e); }); }
      return origEnd.call(this, null, null, cb);
    };
    return stream;
  }

  // --- module exports ----------------------------------------------------
  var fsSync = {
    readFileSync: readFileSync, writeFileSync: writeFileSync, appendFileSync: appendFileSync,
    existsSync: existsSync, statSync: statSync, lstatSync: lstatSync,
    readdirSync: readdirSync, mkdirSync: mkdirSync, rmSync: rmSync, rmdirSync: rmdirSync,
    unlinkSync: unlinkSync, renameSync: renameSync, copyFileSync: copyFileSync,
    realpathSync: realpathSync, chmodSync: chmodSync,
    createReadStream: createReadStream, createWriteStream: createWriteStream,

    // callback-style mirrors
    readFile: callbackified(readFileSync),
    writeFile: callbackified(writeFileSync),
    appendFile: callbackified(appendFileSync),
    exists: function (p, cb) { try { cb && cb(existsSync(p)); } catch (_) { cb && cb(false); } },
    stat: callbackified(statSync), lstat: callbackified(lstatSync),
    readdir: callbackified(readdirSync), mkdir: callbackified(mkdirSync),
    rm: callbackified(rmSync), rmdir: callbackified(rmdirSync),
    unlink: callbackified(unlinkSync), rename: callbackified(renameSync),
    copyFile: callbackified(copyFileSync), realpath: callbackified(realpathSync),
    chmod: callbackified(chmodSync),
    access: function (p, _mode, cb) { if (typeof _mode === "function") { cb = _mode; } cb && (existsSync(p) ? cb(null) : cb(new Error("ENOENT"))); },

    constants: {
      F_OK: 0, R_OK: 4, W_OK: 2, X_OK: 1,
      O_RDONLY: 0, O_WRONLY: 1, O_RDWR: 2, O_CREAT: 64, O_EXCL: 128, O_TRUNC: 512, O_APPEND: 1024,
      S_IFMT: 0o170000, S_IFREG: 0o100000, S_IFDIR: 0o040000,
    },
    promises: null,
  };

  fsSync.promises = {
    readFile: asyncify(readFileSync), writeFile: asyncify(writeFileSync), appendFile: asyncify(appendFileSync),
    stat: asyncify(statSync), lstat: asyncify(lstatSync),
    readdir: asyncify(readdirSync), mkdir: asyncify(mkdirSync),
    rm: asyncify(rmSync), rmdir: asyncify(rmdirSync), unlink: asyncify(unlinkSync),
    rename: asyncify(renameSync), copyFile: asyncify(copyFileSync),
    realpath: asyncify(realpathSync), chmod: asyncify(chmodSync),
    access: function (p) { return existsSync(p) ? Promise.resolve() : Promise.reject(new Error("ENOENT: " + p)); },
  };

  globalThis.__tynd_modules = globalThis.__tynd_modules || Object.create(null);
  globalThis.__tynd_modules["fs"] = fsSync;
  globalThis.__tynd_modules["node:fs"] = fsSync;
  globalThis.__tynd_modules["fs/promises"] = fsSync.promises;
  globalThis.__tynd_modules["node:fs/promises"] = fsSync.promises;
})();
