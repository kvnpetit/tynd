// Shared helpers used by every other polyfill.
//
//   __tynd_to_bytes(BufferSource)        — single canonical Uint8Array view.
//   __tynd_polyfill_handlers (Map)       — prefix → handler(name, data).
//   __tynd_polyfill_event__              — dispatcher invoked by Rust.
//
// Loaded before crypto / fetch / websocket so those polyfills can register
// their handlers with `__tynd_polyfill_handlers.set("fetch", ...)` instead
// of chaining `_prev` references and silently swallowing JSON.parse errors.
(function () {
  globalThis.__tynd_to_bytes = function (source) {
    if (source instanceof Uint8Array) return source;
    if (source instanceof ArrayBuffer) return new Uint8Array(source);
    if (source && source.buffer instanceof ArrayBuffer) {
      return new Uint8Array(source.buffer, source.byteOffset, source.byteLength);
    }
    throw new TypeError("expected BufferSource");
  };

  globalThis.__tynd_polyfill_handlers = new Map();
  globalThis.__tynd_polyfill_event__ = function (name, dataJson) {
    var colonIdx = name.indexOf(":");
    var prefix = colonIdx > 0 ? name.slice(0, colonIdx) : name;
    var handler = globalThis.__tynd_polyfill_handlers.get(prefix);
    if (!handler) return;
    var data;
    try {
      data = JSON.parse(dataJson);
    } catch (e) {
      // Surface decode failures rather than silently dropping them — the
      // previous chained dispatcher swallowed these, which masked Rust-side
      // serialization bugs for whichever polyfill owned the prefix.
      if (typeof console !== "undefined" && console.error) {
        console.error("__tynd_polyfill_event__: bad JSON for '" + name + "': " + e);
      }
      return;
    }
    try {
      handler(name, data);
    } catch (e) {
      if (typeof console !== "undefined" && console.error) {
        console.error("__tynd_polyfill_event__: handler for '" + prefix + "' threw: " + e);
      }
    }
  };
})();
