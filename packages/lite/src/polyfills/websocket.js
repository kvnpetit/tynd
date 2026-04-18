// WebSocket global — WHATWG surface over the native tungstenite bridge.
// Events:  open, message, close, error  (both addEventListener and on* props)
// Readystate: 0 CONNECTING, 1 OPEN, 2 CLOSING, 3 CLOSED
(function () {
  var _seq = 0;
  var _sessions = {};

  function WebSocket(url, protocols) {
    this.url = String(url);
    this.readyState = 0;
    this.binaryType = "arraybuffer";
    this.bufferedAmount = 0;
    this.protocol = "";
    this._listeners = { open: [], message: [], close: [], error: [] };
    this._id = "w" + (++_seq);
    _sessions[this._id] = this;
    try {
      __tynd_ws_connect__(this._id, this.url, JSON.stringify(protocols || []));
    } catch (e) {
      var self = this;
      queueMicrotask(function () { self._fireError(e); self._fireClose(1006, String(e), false); });
    }
  }
  WebSocket.CONNECTING = 0;
  WebSocket.OPEN = 1;
  WebSocket.CLOSING = 2;
  WebSocket.CLOSED = 3;
  WebSocket.prototype.CONNECTING = 0;
  WebSocket.prototype.OPEN = 1;
  WebSocket.prototype.CLOSING = 2;
  WebSocket.prototype.CLOSED = 3;

  WebSocket.prototype.addEventListener = function (type, handler) {
    if (this._listeners[type]) this._listeners[type].push(handler);
  };
  WebSocket.prototype.removeEventListener = function (type, handler) {
    var list = this._listeners[type];
    if (!list) return;
    var i = list.indexOf(handler);
    if (i >= 0) list.splice(i, 1);
  };
  function makeProp(type) {
    Object.defineProperty(WebSocket.prototype, "on" + type, {
      get: function () { return this["_on" + type] || null; },
      set: function (fn) {
        if (this["_on" + type]) this.removeEventListener(type, this["_on" + type]);
        this["_on" + type] = typeof fn === "function" ? fn : null;
        if (this["_on" + type]) this.addEventListener(type, this["_on" + type]);
      },
    });
  }
  makeProp("open"); makeProp("message"); makeProp("close"); makeProp("error");

  WebSocket.prototype._fire = function (type, event) {
    var list = this._listeners[type].slice();
    for (var i = 0; i < list.length; i++) {
      try { list[i].call(this, event); } catch (_) {}
    }
  };
  WebSocket.prototype._fireError = function (err) {
    this._fire("error", { type: "error", message: String(err && err.message || err), target: this });
  };
  WebSocket.prototype._fireClose = function (code, reason, wasClean) {
    if (this.readyState === 3) return;
    this.readyState = 3;
    delete _sessions[this._id];
    this._fire("close", { type: "close", code: code, reason: reason || "", wasClean: !!wasClean, target: this });
  };

  WebSocket.prototype.send = function (data) {
    if (this.readyState !== 1) throw new Error("WebSocket is not OPEN");
    var payload;
    var isBinary = false;
    if (typeof data === "string") {
      payload = data;
    } else {
      var bytes;
      if (data instanceof ArrayBuffer) bytes = new Uint8Array(data);
      else if (data instanceof Uint8Array) bytes = data;
      else if (data && data.buffer instanceof ArrayBuffer) bytes = new Uint8Array(data.buffer, data.byteOffset, data.byteLength);
      else throw new TypeError("WebSocket.send: unsupported data type");
      var s = "";
      for (var i = 0; i < bytes.length; i++) s += String.fromCharCode(bytes[i]);
      payload = btoa(s);
      isBinary = true;
    }
    __tynd_ws_send__(this._id, payload, isBinary);
  };

  WebSocket.prototype.close = function (code, reason) {
    if (this.readyState >= 2) return;
    this.readyState = 2;
    __tynd_ws_close__(this._id, code || 1000, reason || "");
  };

  var _prev = globalThis.__tynd_polyfill_event__;
  globalThis.__tynd_polyfill_event__ = function (name, dataJson) {
    if (name !== "ws:open" && name !== "ws:message" && name !== "ws:close" && name !== "ws:error") {
      if (typeof _prev === "function") return _prev(name, dataJson);
      return;
    }
    var data; try { data = JSON.parse(dataJson); } catch (_) { return; }
    var ws = _sessions[data.id];
    if (!ws) return;
    if (name === "ws:open") {
      ws.readyState = 1;
      ws.protocol = data.protocol || "";
      ws._fire("open", { type: "open", target: ws });
    } else if (name === "ws:message") {
      var payload;
      if (data.isBinary) {
        var s = atob(data.payload);
        var u8 = new Uint8Array(s.length);
        for (var i = 0; i < s.length; i++) u8[i] = s.charCodeAt(i);
        payload = ws.binaryType === "arraybuffer" ? u8.buffer : u8;
      } else {
        payload = data.payload;
      }
      ws._fire("message", { type: "message", data: payload, target: ws });
    } else if (name === "ws:error") {
      ws._fireError(new Error(data.message || "socket error"));
    } else if (name === "ws:close") {
      ws._fireClose(data.code || 1000, data.reason || "", data.wasClean !== false);
    }
  };

  globalThis.WebSocket = WebSocket;
})();
