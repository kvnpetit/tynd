// node:http / node:https — minimal client surface backed by `fetch()`.
// Server (`http.createServer`) is not implemented in lite (no event loop
// exposed for raw sockets); for HTTP servers, see `os.http_server` in
// `@tynd/core/client`. Document explicitly with throw.
(function () {
  var EE = globalThis.EventEmitter;
  var Buffer = globalThis.Buffer;

  function ClientRequest(url, opts, cb) {
    EE.call(this);
    this._chunks = [];
    this._headers = Object.assign({}, (opts && opts.headers) || {});
    this._method = (opts && opts.method) || "GET";
    this._url = url;
    this._aborted = false;
    var self = this;
    if (cb) self.once("response", cb);
    this._endPromise = null;
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
    var body = self._chunks.length ? Buffer.concat(self._chunks.map(function (c) { return c instanceof Uint8Array ? c : Buffer.from(c); })) : undefined;
    var init = { method: self._method, headers: self._headers };
    if (body && self._method !== "GET" && self._method !== "HEAD") init.body = body;
    fetch(self._url, init).then(function (res) {
      var inc = new EE();
      inc.statusCode = res.status;
      inc.statusMessage = res.statusText;
      inc.headers = {};
      res.headers.forEach(function (v, k) { inc.headers[k.toLowerCase()] = v; });
      inc.httpVersion = "1.1";
      inc.setEncoding = function (enc) { inc._encoding = enc; };
      inc.pause = function () {};
      inc.resume = function () {};
      self.emit("response", inc);
      res.arrayBuffer().then(function (buf) {
        var b = Buffer.from(buf);
        var chunk = inc._encoding ? b.toString(inc._encoding) : b;
        inc.emit("data", chunk);
        inc.emit("end");
        inc.emit("close");
      }).catch(function (e) { inc.emit("error", e); });
      if (cb) cb();
    }).catch(function (e) { self.emit("error", e); if (cb) cb(e); });
    return self;
  };
  ClientRequest.prototype.abort = ClientRequest.prototype.destroy = function () { this._aborted = true; this.emit("close"); };
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

  function makeMod(proto) {
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
          "See https://tynd.kvnpetit.com/v1.0/api/os/http"
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
  globalThis.__tynd_modules = globalThis.__tynd_modules || Object.create(null);
  globalThis.__tynd_modules["http"] = makeMod("http:");
  globalThis.__tynd_modules["node:http"] = globalThis.__tynd_modules["http"];
  globalThis.__tynd_modules["https"] = makeMod("https:");
  globalThis.__tynd_modules["node:https"] = globalThis.__tynd_modules["https"];
})();
