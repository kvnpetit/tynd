// Web Streams (ReadableStream / WritableStream / TransformStream) + a thin
// `node:stream` (Readable / Writable / Transform / Duplex / PassThrough)
// adapter. Pragmatic implementation — no async iteration spec quirks, no
// pipeThrough teeing for backpressure beyond a single high-water mark.
//
// Plenty good for what backends actually do with streams: read fetch
// bodies, push chunks to a frontend, transcode them with TextDecoder.
(function () {
  // Always install — QuickJS may ship a partial native impl that lacks
  // WritableStream/TransformStream. Replacing wholesale keeps the surface
  // consistent and matches what `node:stream/web` exports below.
  if (true) {
    function makeQueue() {
      var q = []; var resolvers = []; var closed = false; var error = null;
      return {
        push: function (v) {
          if (closed) return;
          if (resolvers.length) { var r = resolvers.shift(); r.resolve({ value: v, done: false }); }
          else q.push(v);
        },
        close: function () {
          closed = true;
          while (resolvers.length) resolvers.shift().resolve({ value: undefined, done: true });
        },
        fail: function (err) {
          error = err; closed = true;
          while (resolvers.length) resolvers.shift().reject(err);
        },
        next: function () {
          if (q.length) return Promise.resolve({ value: q.shift(), done: false });
          if (error) return Promise.reject(error);
          if (closed) return Promise.resolve({ value: undefined, done: true });
          return new Promise(function (resolve, reject) { resolvers.push({ resolve: resolve, reject: reject }); });
        },
        size: function () { return q.length; },
      };
    }

    function ReadableStream(underlying, _strategy) {
      underlying = underlying || {};
      var q = makeQueue();
      var locked = false;
      var cancelled = false;
      var startResult;
      var pulling = false;
      var controller = {
        enqueue: function (v) { q.push(v); },
        close: function () { q.close(); },
        error: function (e) { q.fail(e); },
        get desiredSize() { return 1 - q.size(); },
      };
      try { startResult = underlying.start ? underlying.start(controller) : undefined; } catch (e) { q.fail(e); }
      function pull() {
        if (pulling || !underlying.pull) return;
        if (q.size() >= 1) return;
        pulling = true;
        Promise.resolve(underlying.pull(controller)).then(
          function () { pulling = false; },
          function (e) { pulling = false; q.fail(e); }
        );
      }
      this._reader = function () {
        if (locked) throw new TypeError("ReadableStream is locked");
        locked = true;
        return {
          read: function () { pull(); return q.next(); },
          releaseLock: function () { locked = false; },
          cancel: function (reason) {
            cancelled = true; q.close();
            if (underlying.cancel) try { return Promise.resolve(underlying.cancel(reason)); } catch (e) { return Promise.reject(e); }
            return Promise.resolve();
          },
          closed: Promise.resolve(),
        };
      };
      this.getReader = this._reader;
      this.cancel = function (reason) { cancelled = true; q.close(); if (underlying.cancel) try { return Promise.resolve(underlying.cancel(reason)); } catch (e) { return Promise.reject(e); } return Promise.resolve(); };
      this.locked = false;
      Object.defineProperty(this, "locked", { get: function () { return locked; } });
      var self = this;
      this[Symbol.asyncIterator] = function () {
        var reader = self.getReader();
        return {
          next: function () { return reader.read(); },
          return: function () { reader.releaseLock(); return Promise.resolve({ value: undefined, done: true }); },
          [Symbol.asyncIterator]: function () { return this; },
        };
      };
      this.tee = function () {
        // Naive tee — buffer everything; OK for backends that read full bodies.
        var bufA = []; var bufB = [];
        var aResolvers = []; var bResolvers = [];
        var done = false;
        var reader = self.getReader();
        function loop() {
          reader.read().then(function (r) {
            if (r.done) {
              done = true;
              while (aResolvers.length) aResolvers.shift().resolve({ value: undefined, done: true });
              while (bResolvers.length) bResolvers.shift().resolve({ value: undefined, done: true });
              return;
            }
            if (aResolvers.length) aResolvers.shift().resolve({ value: r.value, done: false });
            else bufA.push(r.value);
            if (bResolvers.length) bResolvers.shift().resolve({ value: r.value, done: false });
            else bufB.push(r.value);
            loop();
          });
        }
        loop();
        function makeBranch(buf, resolvers) {
          return new ReadableStream({
            pull: function (ctrl) {
              if (buf.length) ctrl.enqueue(buf.shift());
              else if (done) ctrl.close();
              else return new Promise(function (resolve) {
                resolvers.push({ resolve: function (r) {
                  if (r.done) ctrl.close(); else ctrl.enqueue(r.value);
                  resolve();
                } });
              });
            },
          });
        }
        return [makeBranch(bufA, aResolvers), makeBranch(bufB, bResolvers)];
      };
      this.pipeTo = function (writable) {
        var reader = self.getReader();
        var writer = writable.getWriter();
        function pump() {
          return reader.read().then(function (r) {
            if (r.done) return writer.close();
            return writer.write(r.value).then(pump);
          });
        }
        return pump();
      };
      this.pipeThrough = function (pair) { self.pipeTo(pair.writable); return pair.readable; };
    }

    function WritableStream(underlying, _strategy) {
      underlying = underlying || {};
      var ctl = { error: function () {} };
      try { if (underlying.start) underlying.start(ctl); } catch (_) {}
      var locked = false;
      var closed = false;
      this.getWriter = function () {
        if (locked) throw new TypeError("WritableStream is locked");
        locked = true;
        return {
          write: function (chunk) {
            if (closed) return Promise.reject(new TypeError("Stream is closed"));
            if (underlying.write) {
              try { return Promise.resolve(underlying.write(chunk, ctl)); } catch (e) { return Promise.reject(e); }
            }
            return Promise.resolve();
          },
          close: function () {
            if (closed) return Promise.resolve();
            closed = true;
            if (underlying.close) { try { return Promise.resolve(underlying.close()); } catch (e) { return Promise.reject(e); } }
            return Promise.resolve();
          },
          abort: function (reason) {
            closed = true;
            if (underlying.abort) { try { return Promise.resolve(underlying.abort(reason)); } catch (e) { return Promise.reject(e); } }
            return Promise.resolve();
          },
          releaseLock: function () { locked = false; },
          ready: Promise.resolve(),
          closed: Promise.resolve(),
          desiredSize: 1,
        };
      };
      this.locked = false;
      Object.defineProperty(this, "locked", { get: function () { return locked; } });
      this.close = function () { closed = true; if (underlying.close) underlying.close(); return Promise.resolve(); };
      this.abort = function (r) { closed = true; if (underlying.abort) underlying.abort(r); return Promise.resolve(); };
    }

    function TransformStream(transformer, _writableStrategy, _readableStrategy) {
      transformer = transformer || {};
      var queued = [];
      var doneResolvers = [];
      var done = false;
      var ctrl = {
        enqueue: function (chunk) {
          if (doneResolvers.length) { var r = doneResolvers.shift(); r.resolve({ value: chunk, done: false }); }
          else queued.push(chunk);
        },
        terminate: function () {
          done = true;
          while (doneResolvers.length) doneResolvers.shift().resolve({ value: undefined, done: true });
        },
      };
      try { if (transformer.start) transformer.start(ctrl); } catch (_) {}
      this.writable = new WritableStream({
        write: function (chunk) {
          if (transformer.transform) {
            return Promise.resolve(transformer.transform(chunk, ctrl));
          }
          ctrl.enqueue(chunk);
        },
        close: function () {
          if (transformer.flush) Promise.resolve(transformer.flush(ctrl)).then(function () { ctrl.terminate(); });
          else ctrl.terminate();
        },
      });
      this.readable = new ReadableStream({
        pull: function (controller) {
          if (queued.length) controller.enqueue(queued.shift());
          else if (done) controller.close();
          else return new Promise(function (resolve) {
            doneResolvers.push({ resolve: function (r) {
              if (r.done) controller.close(); else controller.enqueue(r.value);
              resolve();
            } });
          });
        },
      });
    }

    globalThis.ReadableStream = ReadableStream;
    globalThis.WritableStream = WritableStream;
    globalThis.TransformStream = TransformStream;
    globalThis.ByteLengthQueuingStrategy = function (opts) { this.highWaterMark = (opts && opts.highWaterMark) || 0; this.size = function (c) { return c.byteLength; }; };
    globalThis.CountQueuingStrategy = function (opts) { this.highWaterMark = (opts && opts.highWaterMark) || 0; this.size = function () { return 1; }; };
  }

  // ---- Node compat: stream.Readable/Writable/Transform/Duplex/PassThrough
  // Built on top of EventEmitter, without trying to be byte-perfect.
  var EE = globalThis.EventEmitter;

  function Readable(opts) {
    EE.call(this);
    this._readable = { buffer: [], ended: false, flowing: null, paused: false, opts: opts || {} };
    var self = this;
    if (opts && typeof opts.read === "function") this._read = opts.read;
  }
  Readable.prototype = Object.create(EE.prototype);
  Readable.prototype._read = function () {};
  Readable.prototype.read = function (_n) {
    var b = this._readable.buffer;
    if (b.length) return b.shift();
    if (this._readable.ended) return null;
    return null;
  };
  Readable.prototype.push = function (chunk) {
    if (chunk === null) {
      this._readable.ended = true;
      this.emit("end");
      this.emit("close");
      return false;
    }
    if (this._readable.flowing) this.emit("data", chunk);
    else this._readable.buffer.push(chunk);
    return true;
  };
  Readable.prototype.pause = function () { this._readable.flowing = false; return this; };
  Readable.prototype.resume = function () {
    this._readable.flowing = true;
    var b = this._readable.buffer;
    while (b.length) this.emit("data", b.shift());
    if (this._readable.ended) this.emit("end");
    return this;
  };
  Readable.prototype.pipe = function (dest) {
    var self = this;
    self.on("data", function (chunk) { dest.write(chunk); });
    self.on("end", function () { dest.end(); });
    return dest;
  };
  Readable.prototype.unpipe = function () { return this; };
  Readable.from = function (iter) {
    var r = new Readable();
    if (iter && typeof iter[Symbol.asyncIterator] === "function") {
      var it = iter[Symbol.asyncIterator]();
      function step() { it.next().then(function (s) { if (s.done) r.push(null); else { r.push(s.value); step(); } }, function (e) { r.emit("error", e); }); }
      step();
    } else if (iter && typeof iter[Symbol.iterator] === "function") {
      for (var v of iter) r.push(v);
      r.push(null);
    }
    return r;
  };

  function Writable(opts) {
    EE.call(this);
    this._writable = { ended: false, opts: opts || {} };
    if (opts && typeof opts.write === "function") this._write = opts.write;
  }
  Writable.prototype = Object.create(EE.prototype);
  Writable.prototype._write = function (_c, _e, cb) { cb && cb(); };
  Writable.prototype.write = function (chunk, encoding, cb) {
    if (typeof encoding === "function") { cb = encoding; encoding = null; }
    var self = this;
    try { self._write(chunk, encoding, function (err) {
      if (err) self.emit("error", err);
      else self.emit("drain");
      if (cb) cb(err);
    }); } catch (e) { self.emit("error", e); if (cb) cb(e); }
    return true;
  };
  Writable.prototype.end = function (chunk, encoding, cb) {
    if (typeof chunk === "function") { cb = chunk; chunk = null; }
    if (chunk != null) this.write(chunk, encoding);
    this._writable.ended = true;
    var self = this;
    Promise.resolve().then(function () { self.emit("finish"); self.emit("close"); if (cb) cb(); });
    return this;
  };

  function Transform(opts) {
    Writable.call(this, opts);
    this._transform = (opts && opts.transform) || function (c, e, cb) { cb(null, c); };
    this._flush = opts && opts.flush;
    var self = this;
    this._write = function (chunk, encoding, cb) {
      self._transform(chunk, encoding, function (err, out) {
        if (out != null) self.emit("data", out);
        cb && cb(err);
      });
    };
  }
  Transform.prototype = Object.create(Writable.prototype);
  Transform.prototype.read = Readable.prototype.read;
  Transform.prototype.pipe = Readable.prototype.pipe;
  Transform.prototype.push = function (chunk) {
    if (chunk === null) { this.emit("end"); return false; }
    this.emit("data", chunk);
    return true;
  };

  function PassThrough(opts) { Transform.call(this, Object.assign({}, opts, { transform: function (c, _e, cb) { cb(null, c); } })); }
  PassThrough.prototype = Object.create(Transform.prototype);

  function Duplex(opts) { Readable.call(this, opts); }
  Duplex.prototype = Object.create(Readable.prototype);
  Object.assign(Duplex.prototype, { write: Writable.prototype.write, end: Writable.prototype.end, _write: Writable.prototype._write });

  function pipeline() {
    var args = Array.prototype.slice.call(arguments);
    var cb = typeof args[args.length - 1] === "function" ? args.pop() : null;
    try {
      for (var i = 0; i < args.length - 1; i++) args[i].pipe(args[i + 1]);
      var last = args[args.length - 1];
      last.on("finish", function () { cb && cb(null); });
      last.on("close", function () { cb && cb(null); });
      last.on("error", function (e) { cb && cb(e); });
    } catch (e) { cb && cb(e); }
    return last;
  }
  function finished(stream, cb) {
    stream.once("end", function () { cb(null); });
    stream.once("finish", function () { cb(null); });
    stream.once("error", function (e) { cb(e); });
  }

  var streamMod = {
    Readable: Readable, Writable: Writable, Transform: Transform, Duplex: Duplex, PassThrough: PassThrough,
    pipeline: pipeline, finished: finished,
    Stream: Readable,
    promises: { pipeline: function () { var args = Array.prototype.slice.call(arguments); return new Promise(function (resolve, reject) { args.push(function (e) { e ? reject(e) : resolve(); }); pipeline.apply(null, args); }); }, finished: function (s) { return new Promise(function (res, rej) { finished(s, function (e) { e ? rej(e) : res(); }); }); } },
    web: {
      ReadableStream: globalThis.ReadableStream,
      WritableStream: globalThis.WritableStream,
      TransformStream: globalThis.TransformStream,
    },
  };
  globalThis.__tynd_modules = globalThis.__tynd_modules || Object.create(null);
  globalThis.__tynd_modules["stream"] = streamMod;
  globalThis.__tynd_modules["node:stream"] = streamMod;
  globalThis.__tynd_modules["stream/promises"] = streamMod.promises;
  globalThis.__tynd_modules["node:stream/promises"] = streamMod.promises;
  globalThis.__tynd_modules["stream/web"] = streamMod.web;
  globalThis.__tynd_modules["node:stream/web"] = streamMod.web;
})();
