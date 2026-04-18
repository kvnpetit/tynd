// AbortController / AbortSignal — minimal WHATWG impl.
(function () {
  function AbortSignal() {
    this.aborted = false;
    this.reason = undefined;
    this._listeners = [];
  }
  AbortSignal.prototype.addEventListener = function (type, handler) {
    if (type !== "abort") return;
    this._listeners.push(handler);
  };
  AbortSignal.prototype.removeEventListener = function (type, handler) {
    if (type !== "abort") return;
    var i = this._listeners.indexOf(handler);
    if (i >= 0) this._listeners.splice(i, 1);
  };
  AbortSignal.prototype.throwIfAborted = function () {
    if (this.aborted) throw this.reason;
  };
  Object.defineProperty(AbortSignal.prototype, "onabort", {
    get: function () { return this._onabort || null; },
    set: function (fn) {
      if (this._onabort) this.removeEventListener("abort", this._onabort);
      this._onabort = typeof fn === "function" ? fn : null;
      if (this._onabort) this.addEventListener("abort", this._onabort);
    },
  });
  AbortSignal.abort = function (reason) {
    var s = new AbortSignal();
    s.aborted = true;
    s.reason = reason === undefined ? new Error("aborted") : reason;
    return s;
  };
  AbortSignal.timeout = function (ms) {
    var s = new AbortSignal();
    setTimeout(function () {
      if (!s.aborted) {
        s.aborted = true;
        s.reason = new Error("timeout");
        var list = s._listeners.slice();
        for (var i = 0; i < list.length; i++) {
          try { list[i]({ type: "abort", target: s }); } catch (_) {}
        }
      }
    }, ms);
    return s;
  };

  function AbortController() {
    this.signal = new AbortSignal();
  }
  AbortController.prototype.abort = function (reason) {
    var s = this.signal;
    if (s.aborted) return;
    s.aborted = true;
    s.reason = reason === undefined ? new Error("aborted") : reason;
    var list = s._listeners.slice();
    for (var i = 0; i < list.length; i++) {
      try { list[i]({ type: "abort", target: s }); } catch (_) {}
    }
  };

  globalThis.AbortController = AbortController;
  globalThis.AbortSignal = AbortSignal;
})();
