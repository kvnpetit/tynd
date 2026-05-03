// node:events — EventEmitter (Node compat) implemented in pure JS.
//
// Covers the shapes npm libs actually use: on / once / off / emit /
// removeListener / removeAllListeners / listenerCount / listeners /
// rawListeners / eventNames / setMaxListeners / getMaxListeners / prependListener.
// `captureRejections` accepted but no-op (sync emit only). `errorMonitor`
// symbol provided so libs that subscribe to it don't blow up.
//
// Exposed via:
//   - globalThis.EventEmitter
//   - globalThis.__tynd_modules['events'] / 'node:events' (consumed by require())
(function () {
  var kErrorMonitor = Symbol("nodejs.events.errorMonitor");
  var kCaptureRejection = Symbol("nodejs.rejection");

  function EventEmitter(opts) {
    EventEmitter.init.call(this, opts);
  }
  EventEmitter.EventEmitter = EventEmitter;
  EventEmitter.errorMonitor = kErrorMonitor;
  EventEmitter.captureRejectionSymbol = kCaptureRejection;
  EventEmitter.defaultMaxListeners = 10;

  EventEmitter.init = function (_opts) {
    if (this._events == null || this._events === Object.getPrototypeOf(this)._events) {
      this._events = Object.create(null);
      this._eventsCount = 0;
    }
    this._maxListeners = this._maxListeners || undefined;
  };

  function _addListener(target, type, listener, prepend) {
    if (typeof listener !== "function") {
      throw new TypeError('"listener" must be a function');
    }
    if (!target._events) EventEmitter.init.call(target);
    var existing = target._events[type];
    if (!existing) {
      target._events[type] = listener;
      target._eventsCount++;
    } else {
      if (typeof existing === "function") {
        existing = target._events[type] = prepend ? [listener, existing] : [existing, listener];
      } else if (prepend) {
        existing.unshift(listener);
      } else {
        existing.push(listener);
      }
    }
    return target;
  }

  EventEmitter.prototype.addListener = function (type, listener) {
    return _addListener(this, type, listener, false);
  };
  EventEmitter.prototype.on = EventEmitter.prototype.addListener;
  EventEmitter.prototype.prependListener = function (type, listener) {
    return _addListener(this, type, listener, true);
  };

  function _onceWrap(target, type, listener) {
    var fired = false;
    function wrapped() {
      if (fired) return;
      fired = true;
      target.removeListener(type, wrapped);
      listener.apply(target, arguments);
    }
    wrapped.listener = listener;
    return wrapped;
  }

  EventEmitter.prototype.once = function (type, listener) {
    if (typeof listener !== "function") throw new TypeError('"listener" must be a function');
    return this.on(type, _onceWrap(this, type, listener));
  };
  EventEmitter.prototype.prependOnceListener = function (type, listener) {
    if (typeof listener !== "function") throw new TypeError('"listener" must be a function');
    return this.prependListener(type, _onceWrap(this, type, listener));
  };

  EventEmitter.prototype.removeListener = function (type, listener) {
    if (!this._events) return this;
    var list = this._events[type];
    if (!list) return this;
    if (list === listener || list.listener === listener) {
      this._eventsCount--;
      delete this._events[type];
    } else if (typeof list !== "function") {
      var idx = -1;
      for (var i = list.length - 1; i >= 0; i--) {
        if (list[i] === listener || list[i].listener === listener) { idx = i; break; }
      }
      if (idx < 0) return this;
      if (list.length === 1) {
        delete this._events[type];
        this._eventsCount--;
      } else {
        list.splice(idx, 1);
        if (list.length === 1) this._events[type] = list[0];
      }
    }
    return this;
  };
  EventEmitter.prototype.off = EventEmitter.prototype.removeListener;

  EventEmitter.prototype.removeAllListeners = function (type) {
    if (!this._events) return this;
    if (arguments.length === 0) {
      this._events = Object.create(null);
      this._eventsCount = 0;
      return this;
    }
    if (this._events[type]) {
      delete this._events[type];
      this._eventsCount--;
    }
    return this;
  };

  EventEmitter.prototype.emit = function (type) {
    if (!this._events) return false;
    var handler = this._events[type];
    if (!handler && type === "error") {
      var err = arguments[1];
      if (err instanceof Error) throw err;
      var e = new Error("Unhandled error." + (err ? " (" + err + ")" : ""));
      e.context = err;
      throw e;
    }
    if (!handler) return false;
    var args = new Array(arguments.length - 1);
    for (var i = 1; i < arguments.length; i++) args[i - 1] = arguments[i];
    if (typeof handler === "function") {
      try { handler.apply(this, args); } catch (e) {
        if (type !== "error") this.emit("error", e); else throw e;
      }
    } else {
      var copy = handler.slice();
      for (var j = 0; j < copy.length; j++) {
        try { copy[j].apply(this, args); } catch (e) {
          if (type !== "error") this.emit("error", e); else throw e;
        }
      }
    }
    return true;
  };

  EventEmitter.prototype.listenerCount = function (type) {
    if (!this._events) return 0;
    var h = this._events[type];
    if (!h) return 0;
    return typeof h === "function" ? 1 : h.length;
  };

  EventEmitter.prototype.listeners = function (type) {
    if (!this._events) return [];
    var h = this._events[type];
    if (!h) return [];
    var arr = typeof h === "function" ? [h] : h.slice();
    return arr.map(function (f) { return f.listener || f; });
  };
  EventEmitter.prototype.rawListeners = function (type) {
    if (!this._events) return [];
    var h = this._events[type];
    if (!h) return [];
    return typeof h === "function" ? [h] : h.slice();
  };

  EventEmitter.prototype.eventNames = function () {
    return this._events ? Object.keys(this._events) : [];
  };

  EventEmitter.prototype.setMaxListeners = function (n) {
    if (typeof n !== "number" || n < 0 || Number.isNaN(n)) {
      throw new RangeError("n must be a non-negative number");
    }
    this._maxListeners = n;
    return this;
  };
  EventEmitter.prototype.getMaxListeners = function () {
    return this._maxListeners == null ? EventEmitter.defaultMaxListeners : this._maxListeners;
  };

  EventEmitter.listenerCount = function (emitter, type) {
    return emitter.listenerCount(type);
  };

  EventEmitter.once = function (emitter, type) {
    return new Promise(function (resolve, reject) {
      function onEvent() {
        emitter.removeListener("error", onError);
        var args = new Array(arguments.length);
        for (var i = 0; i < arguments.length; i++) args[i] = arguments[i];
        resolve(args);
      }
      function onError(err) {
        emitter.removeListener(type, onEvent);
        reject(err);
      }
      emitter.once(type, onEvent);
      if (type !== "error") emitter.once("error", onError);
    });
  };

  EventEmitter.on = function (emitter, type) {
    var unconsumed = [];
    var pending = [];
    var done = false;
    var error = null;
    function push(args) {
      if (pending.length) {
        var p = pending.shift();
        p.resolve({ value: args, done: false });
      } else {
        unconsumed.push(args);
      }
    }
    emitter.on(type, function () {
      var args = new Array(arguments.length);
      for (var i = 0; i < arguments.length; i++) args[i] = arguments[i];
      push(args);
    });
    if (type !== "error") emitter.on("error", function (err) {
      done = true; error = err;
      while (pending.length) pending.shift().reject(err);
    });
    return {
      [Symbol.asyncIterator]: function () { return this; },
      next: function () {
        if (unconsumed.length) return Promise.resolve({ value: unconsumed.shift(), done: false });
        if (done) return error ? Promise.reject(error) : Promise.resolve({ value: undefined, done: true });
        return new Promise(function (resolve, reject) { pending.push({ resolve: resolve, reject: reject }); });
      },
      return: function () { done = true; return Promise.resolve({ value: undefined, done: true }); },
    };
  };

  globalThis.EventEmitter = EventEmitter;
  globalThis.__tynd_modules = globalThis.__tynd_modules || Object.create(null);
  globalThis.__tynd_modules["events"] = EventEmitter;
  globalThis.__tynd_modules["node:events"] = EventEmitter;
})();
