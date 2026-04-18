// fetch / Headers / Request / Response / ReadableStream (body only).
// Native bridges:
//   __tynd_fetch_start__(id, url, optsJson)  -> void, begins async HTTP
//   __tynd_fetch_abort__(id)                  -> void, cancels in-flight
//   __tynd_fetch_deliver__ is set by JS here so Rust can push chunks back:
//     emits "fetch:meta", "fetch:chunk", "fetch:end" via __tynd_os_event__
(function () {
  var _seq = 0;
  var _pending = {};

  function Headers(init) {
    this._m = Object.create(null);
    if (init) {
      if (init instanceof Headers) {
        for (var k in init._m) this._m[k] = init._m[k];
      } else if (Array.isArray(init)) {
        for (var i = 0; i < init.length; i++) this.append(init[i][0], init[i][1]);
      } else if (typeof init === "object") {
        for (var key in init) this.append(key, init[key]);
      }
    }
  }
  Headers.prototype.append = function (k, v) {
    k = String(k).toLowerCase();
    v = String(v);
    this._m[k] = this._m[k] ? this._m[k] + ", " + v : v;
  };
  Headers.prototype.set = function (k, v) {
    this._m[String(k).toLowerCase()] = String(v);
  };
  Headers.prototype.get = function (k) {
    var v = this._m[String(k).toLowerCase()];
    return v == null ? null : v;
  };
  Headers.prototype.has = function (k) {
    return String(k).toLowerCase() in this._m;
  };
  Headers.prototype.delete = function (k) {
    delete this._m[String(k).toLowerCase()];
  };
  Headers.prototype.forEach = function (cb, self) {
    for (var k in this._m) cb.call(self, this._m[k], k, this);
  };
  Headers.prototype.entries = function* () {
    for (var k in this._m) yield [k, this._m[k]];
  };
  Headers.prototype.keys = function* () {
    for (var k in this._m) yield k;
  };
  Headers.prototype.values = function* () {
    for (var k in this._m) yield this._m[k];
  };
  Headers.prototype[Symbol.iterator] = Headers.prototype.entries;

  // Minimal ReadableStream — byte stream backed by a chunk queue.
  function ReadableStream(source) {
    var controller = null;
    var queue = [];
    var waiters = [];
    var closed = false;
    var errored = null;

    function drainTo(resolver) {
      if (queue.length) return resolver({ value: queue.shift(), done: false });
      if (errored) return resolver({ value: undefined, done: true, _err: errored });
      if (closed) return resolver({ value: undefined, done: true });
      waiters.push(resolver);
    }

    controller = {
      enqueue: function (chunk) {
        if (closed || errored) return;
        if (waiters.length) waiters.shift()({ value: chunk, done: false });
        else queue.push(chunk);
      },
      close: function () {
        if (closed) return;
        closed = true;
        while (waiters.length) waiters.shift()({ value: undefined, done: true });
      },
      error: function (e) {
        if (closed || errored) return;
        errored = e || new Error("stream error");
        while (waiters.length) waiters.shift()({ value: undefined, done: true, _err: errored });
      },
    };

    this._controller = controller;
    this._locked = false;

    if (source && typeof source.start === "function") {
      try { source.start(controller); } catch (e) { controller.error(e); }
    }

    this.getReader = function () {
      if (this._locked) throw new TypeError("ReadableStream is locked");
      this._locked = true;
      var self = this;
      return {
        read: function () {
          return new Promise(function (resolve, reject) {
            drainTo(function (r) {
              if (r._err) reject(r._err);
              else resolve({ value: r.value, done: r.done });
            });
          });
        },
        releaseLock: function () { self._locked = false; },
        cancel: function () {
          controller.close();
          self._locked = false;
          return Promise.resolve();
        },
      };
    };

    this[Symbol.asyncIterator] = function () {
      var reader = this.getReader();
      return {
        next: function () { return reader.read(); },
        "return": function () { reader.cancel(); return Promise.resolve({ value: undefined, done: true }); },
        [Symbol.asyncIterator]: function () { return this; },
      };
    };
  }

  function concatBytes(parts) {
    var total = 0;
    for (var i = 0; i < parts.length; i++) total += parts[i].length;
    var out = new Uint8Array(total);
    var off = 0;
    for (var j = 0; j < parts.length; j++) {
      out.set(parts[j], off);
      off += parts[j].length;
    }
    return out;
  }

  function consumeBody(stream) {
    var reader = stream.getReader();
    var chunks = [];
    function pump() {
      return reader.read().then(function (r) {
        if (r.done) return concatBytes(chunks);
        chunks.push(r.value);
        return pump();
      });
    }
    return pump();
  }

  function Response(body, init) {
    init = init || {};
    this.status = init.status == null ? 200 : init.status;
    this.statusText = init.statusText == null ? "" : String(init.statusText);
    this.headers = init.headers instanceof Headers ? init.headers : new Headers(init.headers);
    this.ok = this.status >= 200 && this.status < 300;
    this.redirected = false;
    this.type = "basic";
    this.url = init.url || "";
    this._bodyUsed = false;
    if (body instanceof ReadableStream) {
      this.body = body;
    } else if (body == null) {
      this.body = null;
    } else {
      // Wrap static body (string / Uint8Array) as a closed ReadableStream.
      var enc = typeof body === "string" ? new TextEncoder().encode(body) : body;
      this.body = new ReadableStream({
        start: function (c) { c.enqueue(enc); c.close(); },
      });
    }
  }
  Object.defineProperty(Response.prototype, "bodyUsed", {
    get: function () { return this._bodyUsed; },
  });
  Response.prototype.arrayBuffer = function () {
    if (this._bodyUsed) return Promise.reject(new TypeError("body already used"));
    this._bodyUsed = true;
    if (!this.body) return Promise.resolve(new ArrayBuffer(0));
    return consumeBody(this.body).then(function (u8) {
      var ab = new ArrayBuffer(u8.length);
      new Uint8Array(ab).set(u8);
      return ab;
    });
  };
  Response.prototype.bytes = function () {
    if (this._bodyUsed) return Promise.reject(new TypeError("body already used"));
    this._bodyUsed = true;
    return this.body ? consumeBody(this.body) : Promise.resolve(new Uint8Array(0));
  };
  Response.prototype.text = function () {
    return this.bytes().then(function (u8) { return new TextDecoder().decode(u8); });
  };
  Response.prototype.json = function () {
    return this.text().then(function (s) { return JSON.parse(s); });
  };
  Response.prototype.clone = function () {
    throw new Error("Response.clone: not supported in lite runtime");
  };

  function Request(input, init) {
    if (input instanceof Request) {
      this.url = input.url;
      this.method = init && init.method ? String(init.method).toUpperCase() : input.method;
      this.headers = new Headers(init && init.headers || input.headers);
      this._body = init && init.body !== undefined ? init.body : input._body;
      this.signal = init && init.signal || input.signal || null;
    } else {
      this.url = String(input);
      this.method = init && init.method ? String(init.method).toUpperCase() : "GET";
      this.headers = new Headers(init && init.headers);
      this._body = init && init.body !== undefined ? init.body : null;
      this.signal = init && init.signal || null;
    }
  }

  function bytesToBase64Local(u8) {
    var s = "";
    for (var i = 0; i < u8.length; i++) s += String.fromCharCode(u8[i]);
    return btoa(s);
  }

  // Drain a ReadableStream into a single Uint8Array. Used for streaming
  // upload bodies — we still base64 the whole payload before handing it
  // to ureq, but the JS side can produce it from a stream source.
  function drainReadable(stream) {
    var reader = stream.getReader();
    var chunks = [];
    var total = 0;
    function pump() {
      return reader.read().then(function (r) {
        if (r.done) {
          var out = new Uint8Array(total);
          var off = 0;
          for (var i = 0; i < chunks.length; i++) { out.set(chunks[i], off); off += chunks[i].length; }
          return out;
        }
        var chunk = r.value;
        if (typeof chunk === "string") chunk = new TextEncoder().encode(chunk);
        else if (chunk instanceof ArrayBuffer) chunk = new Uint8Array(chunk);
        else if (!(chunk instanceof Uint8Array) && chunk && chunk.buffer instanceof ArrayBuffer) {
          chunk = new Uint8Array(chunk.buffer, chunk.byteOffset, chunk.byteLength);
        }
        chunks.push(chunk);
        total += chunk.length;
        return pump();
      });
    }
    return pump();
  }

  function encodeBody(body, headers) {
    if (body == null) return Promise.resolve("");
    if (typeof body === "string") {
      if (!headers.has("content-type")) headers.set("content-type", "text/plain;charset=UTF-8");
      return Promise.resolve(btoa(unescape(encodeURIComponent(body))));
    }
    if (body instanceof Uint8Array) return Promise.resolve(bytesToBase64Local(body));
    if (body instanceof ArrayBuffer) return Promise.resolve(bytesToBase64Local(new Uint8Array(body)));
    if (typeof Blob === "function" && body instanceof Blob) {
      if (!headers.has("content-type") && body.type) headers.set("content-type", body.type);
      return body.bytes().then(bytesToBase64Local);
    }
    if (body && typeof body[Symbol.asyncIterator] === "function") {
      return drainReadable(body).then(bytesToBase64Local);
    }
    // URLSearchParams / FormData with simple entries
    if (typeof URLSearchParams === "function" && body instanceof URLSearchParams) {
      if (!headers.has("content-type")) {
        headers.set("content-type", "application/x-www-form-urlencoded;charset=UTF-8");
      }
      var enc = new TextEncoder().encode(body.toString());
      return Promise.resolve(bytesToBase64Local(enc));
    }
    if (typeof FormData === "function" && body instanceof FormData) {
      // Build a multipart/form-data payload with a random boundary. Each
      // entry becomes a part; Blob / File parts carry their own content-type
      // and optional filename.
      var boundary = "----TyndFormBoundary" + Math.random().toString(36).slice(2);
      if (!headers.has("content-type")) {
        headers.set("content-type", "multipart/form-data; boundary=" + boundary);
      }
      var chunks = [];
      var encoder = new TextEncoder();
      var pending = [];
      body.forEach(function (value, name) { pending.push([name, value]); });
      function appendText(s) { chunks.push(encoder.encode(s)); }
      function appendBytes(u8) { chunks.push(u8); }
      function walkEntry(i) {
        if (i >= pending.length) {
          appendText("--" + boundary + "--\r\n");
          var total = 0;
          for (var k = 0; k < chunks.length; k++) total += chunks[k].length;
          var merged = new Uint8Array(total);
          var off = 0;
          for (var j = 0; j < chunks.length; j++) { merged.set(chunks[j], off); off += chunks[j].length; }
          return Promise.resolve(bytesToBase64Local(merged));
        }
        var name = pending[i][0];
        var value = pending[i][1];
        appendText("--" + boundary + "\r\n");
        if (typeof Blob === "function" && value instanceof Blob) {
          var filename = value.name || "blob";
          appendText(
            'Content-Disposition: form-data; name="' + name + '"; filename="' + filename + '"\r\n'
              + "Content-Type: " + (value.type || "application/octet-stream") + "\r\n\r\n",
          );
          return value.bytes().then(function (u8) {
            appendBytes(u8);
            appendText("\r\n");
            return walkEntry(i + 1);
          });
        }
        appendText('Content-Disposition: form-data; name="' + name + '"\r\n\r\n');
        appendText(String(value) + "\r\n");
        return walkEntry(i + 1);
      }
      return walkEntry(0);
    }
    return Promise.reject(new TypeError("fetch: unsupported body type"));
  }

  // Called by Rust via __tynd_polyfill_event__(name, dataJson). Each polyfill
  // that needs native-thread callbacks checks for its own "name:*" prefix.
  var _prev = globalThis.__tynd_polyfill_event__;
  globalThis.__tynd_polyfill_event__ = function (name, dataJson) {
    var data;
    try { data = JSON.parse(dataJson); } catch (_) { return; }
    if (name === "fetch:meta" || name === "fetch:chunk" || name === "fetch:end") {
      var p = _pending[data.id];
      if (!p) return;
      if (name === "fetch:meta") {
        p.resolveHead({
          status: data.status,
          statusText: data.statusText,
          headers: data.headers,
          stream: p.stream,
        });
      } else if (name === "fetch:chunk") {
        var s = atob(data.chunk);
        var u8 = new Uint8Array(s.length);
        for (var i = 0; i < s.length; i++) u8[i] = s.charCodeAt(i);
        p.controller.enqueue(u8);
      } else if (name === "fetch:end") {
        if (data.error) p.controller.error(new Error(data.error));
        else p.controller.close();
        delete _pending[data.id];
      }
      return;
    }
    if (typeof _prev === "function") _prev(name, dataJson);
  };

  globalThis.fetch = function (input, init) {
    var req = input instanceof Request ? input : new Request(input, init);
    if (req.signal && req.signal.aborted) {
      return Promise.reject(req.signal.reason || new Error("aborted"));
    }

    var id = "f" + (++_seq);
    var controller;
    var stream = new ReadableStream({ start: function (c) { controller = c; } });
    var resolveHead, rejectHead;
    var headPromise = new Promise(function (res, rej) {
      resolveHead = res;
      rejectHead = rej;
    });

    _pending[id] = {
      controller: controller,
      stream: stream,
      resolveHead: resolveHead,
      rejectHead: rejectHead,
    };

    if (req.signal) {
      req.signal.addEventListener("abort", function () {
        try { __tynd_fetch_abort__(id); } catch (_) {}
        var p = _pending[id];
        if (p) {
          p.controller.error(req.signal.reason || new Error("aborted"));
          p.rejectHead(req.signal.reason || new Error("aborted"));
          delete _pending[id];
        }
      });
    }

    encodeBody(req._body, req.headers).then(function (bodyB64) {
      var headers = {};
      req.headers.forEach(function (v, k) { headers[k] = v; });
      var opts = { method: req.method, headers: headers, bodyB64: bodyB64 };
      try {
        __tynd_fetch_start__(id, req.url, JSON.stringify(opts));
      } catch (e) {
        delete _pending[id];
        _pending[id] && _pending[id].rejectHead && _pending[id].rejectHead(e);
      }
    }, function (e) {
      var p2 = _pending[id];
      if (p2) { delete _pending[id]; p2.rejectHead(e); }
    });

    return headPromise.then(function (head) {
      var h = new Headers(head.headers || {});
      return new Response(head.stream, {
        status: head.status,
        statusText: head.statusText,
        headers: h,
        url: req.url,
      });
    });
  };

  globalThis.Headers = Headers;
  globalThis.Request = Request;
  globalThis.Response = Response;
  globalThis.ReadableStream = ReadableStream;
})();
