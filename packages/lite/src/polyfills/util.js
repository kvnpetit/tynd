// node:util — inspect / promisify / callbackify / format / types / isDeepStrictEqual.
// Pragmatic surface — covers what 95% of npm uses. No `util.debug`/`util.deprecate`
// wiring (those are no-ops returning the wrapped fn).
(function () {
  function format(fmt) {
    var args = Array.prototype.slice.call(arguments, 1);
    if (typeof fmt !== "string") {
      var out = []; out.push(inspect(fmt));
      for (var i = 0; i < args.length; i++) out.push(inspect(args[i]));
      return out.join(" ");
    }
    var i2 = 0;
    var str = fmt.replace(/%[sdifjoOc%]/g, function (m) {
      if (m === "%%") return "%";
      if (i2 >= args.length) return m;
      var a = args[i2++];
      switch (m) {
        case "%s": return String(a);
        case "%d": case "%i": return Number.isFinite(+a) ? String(parseInt(a, 10)) : "NaN";
        case "%f": return Number.isFinite(+a) ? String(parseFloat(a)) : "NaN";
        case "%j": try { return JSON.stringify(a); } catch (_) { return "[Circular]"; }
        case "%o": case "%O": return inspect(a);
        case "%c": return "";
        default: return m;
      }
    });
    while (i2 < args.length) { str += " " + (typeof args[i2] === "string" ? args[i2] : inspect(args[i2])); i2++; }
    return str;
  }

  function inspect(value, opts) {
    opts = opts || {};
    var depth = typeof opts.depth === "number" ? opts.depth : 2;
    return _inspect(value, depth, new WeakSet());
  }
  function _inspect(v, depth, seen) {
    if (v === null) return "null";
    if (v === undefined) return "undefined";
    var t = typeof v;
    if (t === "string") return "'" + v.replace(/\\/g, "\\\\").replace(/'/g, "\\'") + "'";
    if (t === "number" || t === "boolean" || t === "bigint") return String(v);
    if (t === "symbol") return v.toString();
    if (t === "function") return "[Function: " + (v.name || "anonymous") + "]";
    if (v instanceof Date) return v.toISOString();
    if (v instanceof RegExp) return v.toString();
    if (v instanceof Error) return (v.stack || (v.name + ": " + v.message));
    if (typeof seen.has === "function" && seen.has(v)) return "[Circular]";
    if (typeof seen.add === "function") seen.add(v);
    if (depth < 0) return Array.isArray(v) ? "[Array]" : "[Object]";
    if (Array.isArray(v)) {
      if (!v.length) return "[]";
      var parts = [];
      for (var i = 0; i < v.length; i++) parts.push(_inspect(v[i], depth - 1, seen));
      return "[ " + parts.join(", ") + " ]";
    }
    if (v instanceof Map) {
      var ms = []; v.forEach(function (val, key) { ms.push(_inspect(key, depth - 1, seen) + " => " + _inspect(val, depth - 1, seen)); });
      return "Map(" + v.size + ") { " + ms.join(", ") + " }";
    }
    if (v instanceof Set) {
      var ss = []; v.forEach(function (val) { ss.push(_inspect(val, depth - 1, seen)); });
      return "Set(" + v.size + ") { " + ss.join(", ") + " }";
    }
    if (v instanceof Uint8Array) return "Uint8Array(" + v.length + ") [ " + Array.prototype.slice.call(v, 0, 10).join(", ") + (v.length > 10 ? ", ..." : "") + " ]";
    var keys = Object.keys(v);
    if (!keys.length) return "{}";
    var ps = [];
    for (var j = 0; j < keys.length; j++) ps.push(keys[j] + ": " + _inspect(v[keys[j]], depth - 1, seen));
    return "{ " + ps.join(", ") + " }";
  }
  inspect.colors = {};
  inspect.styles = {};
  inspect.custom = Symbol.for("nodejs.util.inspect.custom");

  function promisify(fn) {
    if (typeof fn !== "function") throw new TypeError("util.promisify: input must be a function");
    function promisified() {
      var args = Array.prototype.slice.call(arguments);
      var self = this;
      return new Promise(function (resolve, reject) {
        try { fn.apply(self, args.concat([function (err, val) { if (err) reject(err); else resolve(val); }])); }
        catch (e) { reject(e); }
      });
    }
    Object.defineProperty(promisified, "name", { value: fn.name + "_promisified" });
    return promisified;
  }
  promisify.custom = Symbol.for("nodejs.util.promisify.custom");

  function callbackify(fn) {
    if (typeof fn !== "function") throw new TypeError("util.callbackify: input must be a function");
    return function () {
      var args = Array.prototype.slice.call(arguments);
      var cb = args.pop();
      if (typeof cb !== "function") throw new TypeError("util.callbackify: last argument must be a function");
      var self = this;
      Promise.resolve(fn.apply(self, args)).then(
        function (val) { cb(null, val); },
        function (err) { cb(err == null ? new Error("Promise was rejected with falsy value") : err); }
      );
    };
  }

  function isDeepStrictEqual(a, b) { return _deepEq(a, b, new WeakMap()); }
  function _deepEq(a, b, seen) {
    if (a === b) return a !== 0 || 1 / a === 1 / b;
    if (a !== a) return b !== b;
    if (typeof a !== "object" || typeof b !== "object" || a === null || b === null) return false;
    if (Object.getPrototypeOf(a) !== Object.getPrototypeOf(b)) return false;
    if (a instanceof Date) return b instanceof Date && a.getTime() === b.getTime();
    if (a instanceof RegExp) return b instanceof RegExp && String(a) === String(b);
    var prev = seen.get(a); if (prev === b) return true;
    seen.set(a, b);
    if (Array.isArray(a)) {
      if (!Array.isArray(b) || a.length !== b.length) return false;
      for (var i = 0; i < a.length; i++) if (!_deepEq(a[i], b[i], seen)) return false;
      return true;
    }
    if (a instanceof Map) {
      if (!(b instanceof Map) || a.size !== b.size) return false;
      var mok = true;
      a.forEach(function (v, k) { if (!_deepEq(v, b.get(k), seen)) mok = false; });
      return mok;
    }
    if (a instanceof Set) {
      if (!(b instanceof Set) || a.size !== b.size) return false;
      var sok = true; a.forEach(function (v) { if (!b.has(v)) sok = false; }); return sok;
    }
    if (a instanceof Uint8Array || a instanceof ArrayBuffer) {
      var av = a instanceof Uint8Array ? a : new Uint8Array(a);
      var bv = b instanceof Uint8Array ? b : (b instanceof ArrayBuffer ? new Uint8Array(b) : null);
      if (!bv || av.byteLength !== bv.byteLength) return false;
      for (var k = 0; k < av.length; k++) if (av[k] !== bv[k]) return false;
      return true;
    }
    var ak = Object.keys(a), bk = Object.keys(b);
    if (ak.length !== bk.length) return false;
    for (var j = 0; j < ak.length; j++) {
      var key = ak[j];
      if (!Object.prototype.hasOwnProperty.call(b, key)) return false;
      if (!_deepEq(a[key], b[key], seen)) return false;
    }
    return true;
  }

  var types = {
    isAnyArrayBuffer: function (v) { return v instanceof ArrayBuffer || (typeof SharedArrayBuffer !== "undefined" && v instanceof SharedArrayBuffer); },
    isArrayBuffer: function (v) { return v instanceof ArrayBuffer; },
    isArrayBufferView: function (v) { return ArrayBuffer.isView(v); },
    isAsyncFunction: function (v) { return typeof v === "function" && Object.prototype.toString.call(v) === "[object AsyncFunction]"; },
    isBigInt64Array: function (v) { return typeof BigInt64Array !== "undefined" && v instanceof BigInt64Array; },
    isBigUint64Array: function (v) { return typeof BigUint64Array !== "undefined" && v instanceof BigUint64Array; },
    isBooleanObject: function (v) { return typeof v === "object" && Object.prototype.toString.call(v) === "[object Boolean]"; },
    isDataView: function (v) { return v instanceof DataView; },
    isDate: function (v) { return v instanceof Date; },
    isFloat32Array: function (v) { return v instanceof Float32Array; },
    isFloat64Array: function (v) { return v instanceof Float64Array; },
    isGeneratorFunction: function (v) { return typeof v === "function" && Object.prototype.toString.call(v) === "[object GeneratorFunction]"; },
    isInt8Array: function (v) { return v instanceof Int8Array; },
    isInt16Array: function (v) { return v instanceof Int16Array; },
    isInt32Array: function (v) { return v instanceof Int32Array; },
    isMap: function (v) { return v instanceof Map; },
    isNativeError: function (v) { return v instanceof Error; },
    isNumberObject: function (v) { return typeof v === "object" && Object.prototype.toString.call(v) === "[object Number]"; },
    isPromise: function (v) { return v != null && typeof v.then === "function"; },
    isRegExp: function (v) { return v instanceof RegExp; },
    isSet: function (v) { return v instanceof Set; },
    isStringObject: function (v) { return typeof v === "object" && Object.prototype.toString.call(v) === "[object String]"; },
    isSymbolObject: function (v) { return typeof v === "object" && Object.prototype.toString.call(v) === "[object Symbol]"; },
    isTypedArray: function (v) { return ArrayBuffer.isView(v) && !(v instanceof DataView); },
    isUint8Array: function (v) { return v instanceof Uint8Array; },
    isUint8ClampedArray: function (v) { return v instanceof Uint8ClampedArray; },
    isUint16Array: function (v) { return v instanceof Uint16Array; },
    isUint32Array: function (v) { return v instanceof Uint32Array; },
    isWeakMap: function (v) { return v instanceof WeakMap; },
    isWeakSet: function (v) { return v instanceof WeakSet; },
  };

  function deprecate(fn, _msg) { return fn; }
  function inherits(ctor, superCtor) {
    if (ctor && superCtor) {
      ctor.super_ = superCtor;
      Object.setPrototypeOf(ctor.prototype, superCtor.prototype);
    }
  }

  var util = {
    format: format,
    formatWithOptions: function (_o, fmt) { var a = Array.prototype.slice.call(arguments, 1); return format.apply(null, a); },
    inspect: inspect,
    promisify: promisify,
    callbackify: callbackify,
    isDeepStrictEqual: isDeepStrictEqual,
    types: types,
    deprecate: deprecate,
    inherits: inherits,
    debuglog: function () { return function () {}; },
    debug: function () { return function () {}; },
    TextEncoder: globalThis.TextEncoder,
    TextDecoder: globalThis.TextDecoder,
    isArray: Array.isArray,
    isBoolean: function (v) { return typeof v === "boolean"; },
    isNull: function (v) { return v === null; },
    isNullOrUndefined: function (v) { return v == null; },
    isNumber: function (v) { return typeof v === "number"; },
    isString: function (v) { return typeof v === "string"; },
    isUndefined: function (v) { return v === undefined; },
    isObject: function (v) { return v !== null && typeof v === "object"; },
    isFunction: function (v) { return typeof v === "function"; },
    isPrimitive: function (v) { return v == null || (typeof v !== "object" && typeof v !== "function"); },
    isError: function (v) { return v instanceof Error; },
    isRegExp: function (v) { return v instanceof RegExp; },
    isDate: function (v) { return v instanceof Date; },
    isBuffer: function (v) { return globalThis.Buffer && globalThis.Buffer.isBuffer && globalThis.Buffer.isBuffer(v); },
    log: function (msg) { console.log(msg); },
    _extend: function (a, b) { return Object.assign(a, b); },
  };

  globalThis.__tynd_modules = globalThis.__tynd_modules || Object.create(null);
  globalThis.__tynd_modules["util"] = util;
  globalThis.__tynd_modules["node:util"] = util;
  globalThis.__tynd_modules["util/types"] = types;
  globalThis.__tynd_modules["node:util/types"] = types;
})();
