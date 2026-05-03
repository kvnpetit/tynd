// Bundles unenv (https://github.com/unjs/unenv) Node compat layer into a
// single self-registering JS file consumed by tynd-lite at QuickJS startup.
//
// Output: `../polyfills-built/unenv-bundle.js`
//
// Strategy: each Node builtin module (buffer, fs, process, stream, …) is
// bundled as an IIFE that writes its exports into `globalThis.__tynd_modules`
// under both bare ("fs") and prefixed ("node:fs") names. The existing
// `require()` shim in `src/polyfills/require.js` resolves IDs against this
// registry — so end-users keep doing `import fs from "node:fs"` (rewritten
// to `require()` by the bundler) and unenv's polyfill answers.
//
// Tynd-specific native bridges (cap-std fs, ureq HTTP, blake3 crypto) are
// patched on top of the unenv exports in `src/polyfills/tynd_overrides.js`,
// which runs AFTER this bundle. unenv supplies the API shape; Tynd swaps
// the slow stub internals for native perf.
//
// Build deps (esbuild) are dev-only — no runtime impact. The compiled JS
// is checked into `polyfills-built/` so `cargo build` doesn't need bun on
// every machine; CI re-runs this script + asserts the output is up to date.

import { build } from "esbuild";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const OUT_DIR = resolve(HERE, "..", "polyfills-built");
const OUT_FILE = resolve(OUT_DIR, "unenv-bundle.js");

// Match the `nodeless` preset in unenv/dist/index.cjs — every Node builtin
// it can actually polyfill (vs the proxy-mock for the rest). When a new
// unenv release adds modules (e.g. `node:zlib` shipped with real impl),
// extend this list.
//
// The mapping `{ id: requirePath }` — `id` is what userland imports,
// `requirePath` is the unenv source.
const NODE_MODULES: Record<string, string> = {
  events: "unenv/runtime/node/events/index",
  buffer: "unenv/runtime/node/buffer/index",
  process: "unenv/runtime/node/process/index",
  // path: skipped — unenv aliases `path.win32` to `pathe` which is
  // posix-only. Tynd ships a custom `path.js` that does real win32
  // separator/drive handling for cross-platform code.
  os: "unenv/runtime/node/os/index",
  url: "unenv/runtime/node/url/index",
  util: "unenv/runtime/node/util/index",
  "util/types": "unenv/runtime/node/util/types/index",
  fs: "unenv/runtime/node/fs/index",
  "fs/promises": "unenv/runtime/node/fs/promises/index",
  stream: "unenv/runtime/node/stream/index",
  "stream/promises": "unenv/runtime/node/stream/promises/index",
  "stream/consumers": "unenv/runtime/node/stream/consumers/index",
  "stream/web": "unenv/runtime/node/stream/web/index",
  string_decoder: "unenv/runtime/node/string_decoder/index",
  crypto: "unenv/runtime/node/crypto/index",
  http: "unenv/runtime/node/http/index",
  https: "unenv/runtime/node/https/index",
  net: "unenv/runtime/node/net/index",
  async_hooks: "unenv/runtime/node/async_hooks/index",
  module: "unenv/runtime/node/module/index",
};

// Modules with no real impl in unenv but commonly imported. We register
// `unenv/runtime/mock/proxy-cjs` (returns a Proxy that no-ops every access)
// so `require("dns")` doesn't blow up — calls to actual methods throw.
const MOCK_MODULES = [
  "assert",
  "assert/strict",
  "child_process",
  "cluster",
  "console",
  "constants",
  "dgram",
  "diagnostics_channel",
  "dns",
  "dns/promises",
  "domain",
  "http2",
  "perf_hooks",
  "punycode",
  "querystring", // unenv has no polyfill — keep custom one for now
  "readline",
  "repl",
  "sys",
  "timers",
  "timers/promises",
  "tls",
  "tty",
  "v8",
  "vm",
  "wasi",
  "worker_threads",
  "zlib",
  "trace_events",
  "inspector",
];

async function buildOne(id: string, entry: string): Promise<string> {
  // Each module is bundled in IIFE format → returns its module.exports.
  // We emit a JS string of the form:
  //   __tynd_modules["fs"] = (function(){ /* bundled iife */ })();
  //   __tynd_modules["node:fs"] = __tynd_modules["fs"];
  //
  // `globalName` is what the IIFE assigns to `var globalName = ...`; we
  // wrap the result through Object.assign so default + named exports
  // both work like CJS.
  const result = await build({
    entryPoints: [entry],
    bundle: true,
    format: "iife",
    globalName: "__tynd_mod_export",
    write: false,
    platform: "neutral",
    target: ["es2020"],
    treeShaking: true,
    minify: false, // readability — final blob is zstd-compressed anyway
    legalComments: "none",
    logLevel: "warning",
    // unenv re-exports `node-fetch-native/polyfill` etc. — let esbuild
    // resolve them via NODE_PATH (peer deps in node_modules/).
    resolveExtensions: [".mjs", ".js", ".cjs"],
    mainFields: ["module", "main"],
    conditions: ["import", "default"],
    // Keep these external — they're either provided by us (`globalThis.fetch`)
    // or resolved at runtime via the registry. esbuild would otherwise inline
    // every node-fetch byte just to satisfy unenv's optional polyfill imports.
    external: [],
  });
  const code = result.outputFiles[0]!.text;
  // The IIFE assigns to var __tynd_mod_export — we want the module's full
  // namespace (default + named). esbuild emits something like:
  //   var __tynd_mod_export = (() => { var x = {}; ... return __toCommonJS(x); })();
  // The CommonJS interop adds a `default` field iff there was an `export default`.
  // We unwrap it: if `__tynd_mod_export.default` exists AND has a wide surface
  // (more than just `{ default }`), prefer it; otherwise use the namespace.
  return [
    `// === ${id} ===`,
    code,
    // CJS interop: if there's a default export with its own properties (the common
    // shape for node:* polyfills), expose default; else the namespace.
    `(function(){`,
    `  var ns = __tynd_mod_export;`,
    `  var def = ns && ns.default;`,
    `  var mod;`,
    `  if (def && typeof def === "object") {`,
    // Merge named exports onto default so both `import fs from "fs"` and
    // `import { readFile } from "fs"` work after CJS resolution.
    `    mod = Object.assign({}, def);`,
    `    for (var k in ns) {`,
    `      if (k !== "default" && !(k in mod)) mod[k] = ns[k];`,
    `    }`,
    `  } else {`,
    `    mod = ns;`,
    `  }`,
    `  globalThis.__tynd_modules = globalThis.__tynd_modules || Object.create(null);`,
    `  globalThis.__tynd_modules[${JSON.stringify(id)}] = mod;`,
    `  globalThis.__tynd_modules[${JSON.stringify("node:" + id)}] = mod;`,
    `})();`,
    "",
  ].join("\n");
}

async function buildMockProxy(): Promise<string> {
  // unenv ships a single Proxy mock that responds to any access with a
  // no-op fn. Useful for `require("dns")` etc. without bloating the bundle.
  const result = await build({
    entryPoints: [resolve(HERE, "node_modules/unenv/runtime/mock/proxy.mjs")],
    bundle: true,
    format: "iife",
    globalName: "__tynd_mock_proxy",
    write: false,
    platform: "neutral",
    target: ["es2020"],
    treeShaking: true,
    minify: false,
    legalComments: "none",
    logLevel: "warning",
  });
  const code = result.outputFiles[0]!.text;
  const registrations = MOCK_MODULES.map(
    (m) =>
      `globalThis.__tynd_modules[${JSON.stringify(m)}] = mock; ` +
      `globalThis.__tynd_modules[${JSON.stringify("node:" + m)}] = mock;`,
  ).join("\n  ");
  return [
    `// === mock proxy (covers unimplemented builtins) ===`,
    code,
    `(function(){`,
    `  var mock = (__tynd_mock_proxy && __tynd_mock_proxy.default) || __tynd_mock_proxy;`,
    `  globalThis.__tynd_modules = globalThis.__tynd_modules || Object.create(null);`,
    `  ${registrations}`,
    `})();`,
    "",
  ].join("\n");
}

async function buildGlobals(): Promise<string> {
  // Wire the unenv globals: `globalThis.process` and `globalThis.Buffer`
  // come from the corresponding modules registered above. This runs AFTER
  // the per-module IIFEs so the registry is populated.
  return [
    `// === globals (process, Buffer) ===`,
    `(function(){`,
    `  var modules = globalThis.__tynd_modules || {};`,
    `  if (typeof globalThis.process === "undefined" && modules.process) {`,
    `    globalThis.process = modules.process;`,
    `  }`,
    `  if (typeof globalThis.Buffer === "undefined" && modules.buffer && modules.buffer.Buffer) {`,
    `    globalThis.Buffer = modules.buffer.Buffer;`,
    `  }`,
    `  // Node-style timer aliases that some npm packages reach for.`,
    `  if (typeof globalThis.setImmediate === "undefined") {`,
    `    globalThis.setImmediate = function (fn) {`,
    `      var args = Array.prototype.slice.call(arguments, 1);`,
    `      return setTimeout(function(){ fn.apply(null, args); }, 0);`,
    `    };`,
    `  }`,
    `  if (typeof globalThis.clearImmediate === "undefined") {`,
    `    globalThis.clearImmediate = clearTimeout;`,
    `  }`,
    `  // Node global alias — many npm packages assume \`global === globalThis\`.`,
    `  if (typeof globalThis.global === "undefined") {`,
    `    globalThis.global = globalThis;`,
    `  }`,
    `})();`,
    "",
  ].join("\n");
}

async function buildWebStreams(): Promise<string> {
  // Install ReadableStream / WritableStream / TransformStream + friends
  // on globalThis. unenv's `node:stream/web` module reads these from
  // globalThis at module-load time and falls back to notImplemented
  // stubs otherwise — so this MUST run before the unenv bundle.
  //
  // The polyfill ships an IIFE that auto-installs to globalThis, which
  // is exactly what we want.
  const code = await readFile(
    resolve(HERE, "node_modules/web-streams-polyfill/dist/polyfill.js"),
    "utf8",
  );
  return [
    `// === web-streams-polyfill (installs ReadableStream / WritableStream / TransformStream globals) ===`,
    code,
    "",
  ].join("\n");
}

async function main() {
  await mkdir(OUT_DIR, { recursive: true });

  const parts: string[] = [
    "// AUTO-GENERATED by polyfills-src/build-polyfills.ts. DO NOT EDIT.",
    "// Source: unenv " + (await unenvVersion()),
    "// Re-run: cd polyfills-src && bun run build",
    "",
    "(function(){",
    "  globalThis.__tynd_modules = globalThis.__tynd_modules || Object.create(null);",
    // unenv@1.10.0 buffer/index.mjs references bare `global.btoa` instead
    // of `globalThis.btoa` — install the alias BEFORE any module evaluates
    // or `global is not defined` blows up at first IIFE.
    "  if (typeof globalThis.global === 'undefined') globalThis.global = globalThis;",
    "  if (typeof globalThis.self === 'undefined') globalThis.self = globalThis;",
    // unenv's process polyfill compares `cachedSetTimeout === setTimeout`
    // (bare ref) inside `runTimeout`. If setTimeout is undefined that's a
    // ReferenceError. The tynd-lite worker installs real timers via
    // `quickjs/globals.js` BEFORE polyfills, so this branch only runs in
    // contexts where someone embeds the polyfill bundle without timers
    // (notably the polyfill smoke tests). Provide harmless stubs.
    "  if (typeof globalThis.setTimeout === 'undefined') globalThis.setTimeout = function(){ return 0; };",
    "  if (typeof globalThis.clearTimeout === 'undefined') globalThis.clearTimeout = function(){};",
    "  if (typeof globalThis.setInterval === 'undefined') globalThis.setInterval = function(){ return 0; };",
    "  if (typeof globalThis.clearInterval === 'undefined') globalThis.clearInterval = function(){};",
    // unenv util/types references SharedArrayBuffer at top-level for an
    // `instanceof` check. QuickJS doesn't ship it; stub it as a noop class
    // so `val instanceof SharedArrayBuffer` returns false instead of
    // throwing.
    "  if (typeof globalThis.SharedArrayBuffer === 'undefined') globalThis.SharedArrayBuffer = function SharedArrayBuffer(){};",
    "})();",
    "",
    // Web Streams must be installed BEFORE unenv (node:stream/web reads
    // them at module-load time).
    await buildWebStreams(),
  ];

  // Buffer first — fs / stream / crypto need globalThis.Buffer wired before
  // their bodies execute (they read Buffer at module load to set up class
  // statics). process before path/os (path branches on process.platform).
  const order = [
    "buffer",
    "process",
    "events",
    "stream",
    "stream/promises",
    "stream/consumers",
    "stream/web",
    "string_decoder",
    "util",
    "util/types",
    "os",
    "url",
    "fs",
    "fs/promises",
    "crypto",
    "http",
    "https",
    "net",
    "async_hooks",
    "module",
  ];

  // Wire globals (process, Buffer) right after their modules register so
  // anything later in the pipeline can read globalThis.process.platform etc.
  for (const id of ["buffer", "process"]) {
    const entry = NODE_MODULES[id];
    if (!entry) continue;
    parts.push(await buildOne(id, entry));
  }
  parts.push(await buildGlobals());

  for (const id of order.slice(2)) {
    const entry = NODE_MODULES[id];
    if (!entry) continue;
    parts.push(await buildOne(id, entry));
  }

  parts.push(await buildMockProxy());

  const out = parts.join("\n");
  await writeFile(OUT_FILE, out);
  const sizeKb = (out.length / 1024).toFixed(1);
  console.log(`unenv bundle: ${OUT_FILE} (${sizeKb} KB raw)`);
}

async function unenvVersion(): Promise<string> {
  const pkg = JSON.parse(
    await readFile(resolve(HERE, "node_modules/unenv/package.json"), "utf8"),
  );
  return pkg.version;
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
