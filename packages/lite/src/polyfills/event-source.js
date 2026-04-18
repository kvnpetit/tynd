// EventSource (Server-Sent Events) — implemented on top of fetch streaming.
// Good-enough subset: open/message/error + custom event types via "event:" lines.
// Does not implement auto-reconnect (Last-Event-ID) — apps that need it should
// wrap EventSource and handle reconnection explicitly.
(function () {
  function EventSource(url, init) {
    this.url = String(url);
    this.readyState = 0;
    this.withCredentials = !!(init && init.withCredentials);
    this._listeners = { open: [], message: [], error: [] };
    this._controller = new AbortController();
    var self = this;

    fetch(this.url, {
      method: "GET",
      headers: { "accept": "text/event-stream", "cache-control": "no-cache" },
      signal: this._controller.signal,
    }).then(function (res) {
      if (!res.ok) {
        self.readyState = 2;
        self._fire("error", { type: "error" });
        return;
      }
      self.readyState = 1;
      self._fire("open", { type: "open" });
      return self._pump(res.body.getReader());
    }).catch(function () {
      self.readyState = 2;
      self._fire("error", { type: "error" });
    });
  }

  EventSource.CONNECTING = 0;
  EventSource.OPEN = 1;
  EventSource.CLOSED = 2;
  EventSource.prototype.CONNECTING = 0;
  EventSource.prototype.OPEN = 1;
  EventSource.prototype.CLOSED = 2;

  EventSource.prototype.addEventListener = function (type, handler) {
    if (!this._listeners[type]) this._listeners[type] = [];
    this._listeners[type].push(handler);
  };
  EventSource.prototype.removeEventListener = function (type, handler) {
    var list = this._listeners[type];
    if (!list) return;
    var i = list.indexOf(handler);
    if (i >= 0) list.splice(i, 1);
  };
  ["open", "message", "error"].forEach(function (type) {
    Object.defineProperty(EventSource.prototype, "on" + type, {
      get: function () { return this["_on" + type] || null; },
      set: function (fn) {
        if (this["_on" + type]) this.removeEventListener(type, this["_on" + type]);
        this["_on" + type] = typeof fn === "function" ? fn : null;
        if (this["_on" + type]) this.addEventListener(type, this["_on" + type]);
      },
    });
  });

  EventSource.prototype._fire = function (type, event) {
    var list = (this._listeners[type] || []).slice();
    for (var i = 0; i < list.length; i++) {
      try { list[i].call(this, event); } catch (_) {}
    }
  };

  EventSource.prototype._pump = function (reader) {
    var self = this;
    var decoder = new TextDecoder();
    var buf = "";
    var eventType = "message";
    var dataLines = [];
    var lastId = "";

    function dispatch() {
      if (dataLines.length === 0) { eventType = "message"; return; }
      var data = dataLines.join("\n");
      self._fire(eventType, { type: eventType, data: data, lastEventId: lastId, origin: "" });
      eventType = "message";
      dataLines = [];
    }

    function loop() {
      return reader.read().then(function (r) {
        if (r.done) { self.close(); return; }
        buf += decoder.decode(r.value, { stream: true });
        var idx;
        while ((idx = buf.indexOf("\n")) >= 0) {
          var line = buf.slice(0, idx);
          buf = buf.slice(idx + 1);
          if (line.endsWith("\r")) line = line.slice(0, -1);

          if (line === "") { dispatch(); continue; }
          if (line.charAt(0) === ":") continue; // comment
          var colon = line.indexOf(":");
          var field = colon < 0 ? line : line.slice(0, colon);
          var value = colon < 0 ? "" : line.slice(colon + 1);
          if (value.charAt(0) === " ") value = value.slice(1);
          if (field === "data") dataLines.push(value);
          else if (field === "event") eventType = value || "message";
          else if (field === "id") lastId = value;
          // "retry" intentionally ignored — no auto-reconnect.
        }
        return loop();
      });
    }
    return loop();
  };

  EventSource.prototype.close = function () {
    if (this.readyState === 2) return;
    this.readyState = 2;
    try { this._controller.abort(); } catch (_) {}
  };

  globalThis.EventSource = EventSource;
})();
