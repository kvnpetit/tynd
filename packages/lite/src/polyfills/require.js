// CommonJS-style `require()` shim — resolves builtin / node: prefixed
// modules from the `__tynd_modules` registry. User-space CJS resolution
// (npm packages from disk) is handled at *bundle time* by the tynd CLI;
// the embedded backend never reads `node_modules` at runtime.
//
// Behaviour:
//   - `require("fs")`, `require("node:fs")`, `require("path")` etc. work.
//   - require.cache mirrors what the bundler will have produced statically.
//   - Native binding paths (.node files) throw a helpful error explaining
//     the QuickJS / V8 ABI incompatibility + suggesting Tynd plugins.
//   - Provides __dirname / __filename / module / exports placeholders for
//     code that probes them. The real values are wired by the bundler per
//     CJS module wrapper.
(function () {
  var modules = (globalThis.__tynd_modules = globalThis.__tynd_modules || Object.create(null));
  var cache = Object.create(null);

  function require(id) {
    if (typeof id !== "string") throw new TypeError("require(): id must be a string, got " + typeof id);
    if (cache[id]) return cache[id].exports;
    // Stripped form e.g. "node:fs" -> "fs" lookup so both shapes resolve.
    var m = modules[id];
    if (m === undefined && id.startsWith("node:")) m = modules[id.slice(5)];
    if (m === undefined && !id.startsWith("node:")) m = modules["node:" + id];
    if (m !== undefined) {
      cache[id] = { exports: m, loaded: true, id: id };
      return m;
    }
    if (id.endsWith(".node") || /\.node($|\?)/.test(id)) {
      throw new Error(
        "require('" + id + "'): native bindings (.node files) require a V8/JSC ABI; tynd-lite uses QuickJS. " +
        "Drop-in plugin alternatives:\n" +
        "  - sharp / image-codec → @tynd/plugin-image\n" +
        "  - better-sqlite3      → @tynd/plugin-sql\n" +
        "  - node-pty            → @tynd/plugin-terminal\n" +
        "  - bcrypt / argon2     → globalThis.crypto.subtle (Web Crypto) or @noble/hashes\n" +
        "See https://tynd.kvnpetit.com/v1.0/guides/native-plugins-mapping for the full list."
      );
    }
    throw new Error(
      "Cannot find module '" + id + "' in tynd-lite runtime. " +
      "Userland packages must be statically bundled by `tynd build` — see https://tynd.kvnpetit.com/v1.0/guides/bundling. " +
      "If '" + id + "' is a Node builtin you expected to find, file an issue at github.com/tynd-apps/tynd."
    );
  }
  require.cache = cache;
  require.resolve = function (id) {
    if (modules[id] !== undefined) return id;
    if (id.startsWith("node:") && modules[id.slice(5)] !== undefined) return id;
    if (modules["node:" + id] !== undefined) return "node:" + id;
    throw new Error("Cannot resolve '" + id + "'");
  };
  require.resolve.paths = function () { return null; };
  require.extensions = Object.create(null);
  require.main = undefined;

  globalThis.require = require;
  // Module/exports placeholders — bundlers usually inject per-file shims
  // around these. Setting them at the top level lets bare CJS scripts run.
  if (typeof globalThis.module === "undefined") {
    globalThis.module = { exports: {}, id: ".", filename: "<lite-bundle>", parent: null, children: [], loaded: false, paths: [] };
  }
  if (typeof globalThis.exports === "undefined") globalThis.exports = globalThis.module.exports;
  if (typeof globalThis.__filename === "undefined") globalThis.__filename = "<lite-bundle>";
  if (typeof globalThis.__dirname === "undefined") globalThis.__dirname = "/";
})();
