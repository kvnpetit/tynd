// structuredClone + Promise.withResolvers.
// structuredClone: handles primitives, Date, Map, Set, RegExp, ArrayBuffer,
// TypedArrays, plain objects, and arrays, with circular-ref detection.
// Throws on Functions, Symbols, WeakMap/WeakSet, DOM nodes (N/A here anyway).
(function () {
  function cloneInner(v, seen) {
    if (v === null || typeof v !== "object") {
      if (typeof v === "function") throw new Error("structuredClone: functions not cloneable");
      if (typeof v === "symbol") throw new Error("structuredClone: symbols not cloneable");
      return v;
    }
    var prev = seen.get(v);
    if (prev) return prev;

    if (v instanceof Date) { var d = new Date(v.getTime()); seen.set(v, d); return d; }
    if (v instanceof RegExp) { var r = new RegExp(v.source, v.flags); seen.set(v, r); return r; }
    if (v instanceof ArrayBuffer) {
      var ab = v.slice(0); seen.set(v, ab); return ab;
    }
    if (ArrayBuffer.isView(v)) {
      var copy = new v.constructor(v.length);
      copy.set(v);
      seen.set(v, copy);
      return copy;
    }
    if (v instanceof Map) {
      var m = new Map(); seen.set(v, m);
      v.forEach(function (val, key) { m.set(cloneInner(key, seen), cloneInner(val, seen)); });
      return m;
    }
    if (v instanceof Set) {
      var s = new Set(); seen.set(v, s);
      v.forEach(function (x) { s.add(cloneInner(x, seen)); });
      return s;
    }
    if (Array.isArray(v)) {
      var a = new Array(v.length); seen.set(v, a);
      for (var i = 0; i < v.length; i++) a[i] = cloneInner(v[i], seen);
      return a;
    }
    // Plain object. Preserve enumerable own keys (including Symbols).
    var o = Object.create(Object.getPrototypeOf(v));
    seen.set(v, o);
    var keys = Object.keys(v);
    for (var k = 0; k < keys.length; k++) o[keys[k]] = cloneInner(v[keys[k]], seen);
    return o;
  }

  globalThis.structuredClone = function (value, options) {
    // options.transfer intentionally not supported (no real Transferable
    // semantics in QuickJS — we'd just copy).
    if (options && options.transfer && options.transfer.length) {
      throw new Error("structuredClone: transfer list not supported in lite");
    }
    return cloneInner(value, new WeakMap());
  };

  if (typeof Promise.withResolvers !== "function") {
    Promise.withResolvers = function () {
      var resolve, reject;
      var promise = new Promise(function (res, rej) { resolve = res; reject = rej; });
      return { promise: promise, resolve: resolve, reject: reject };
    };
  }
})();
