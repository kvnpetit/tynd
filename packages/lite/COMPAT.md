# tynd-lite Runtime Compatibility Matrix

`tynd-lite` is the embedded QuickJS-based backend runtime used by Tynd
apps that opt out of the Bun subprocess. It ships with a comprehensive
polyfill layer so the bulk of Node + Bun + Web ecosystem code runs
unmodified.

This document is the canonical "does it work?" reference. Statuses:

| Symbol  | Meaning                                                         |
|---------|-----------------------------------------------------------------|
| OK      | Full parity with Node/Bun/Web. Drop-in.                         |
| PARTIAL | Most-used surface works; edge cases or perf differ. Documented. |
| SHIM    | API present; throws helpful redirect to Tynd plugin / native.   |
| NO      | Structural incompat. Use the listed alternative.                |

**Powered by [`unenv`](https://github.com/unjs/unenv) (UnJS)** —
the same Node compat layer Cloudflare Workers, Nitro, and Nuxt use in
production. Tynd bundles unenv's `nodeless` preset offline (see
`polyfills-src/build-polyfills.ts`) and patches the API stubs with native
bridges (cap-std fs, ureq HTTP, blake3 crypto, real OS env/argv/cwd) so
fs / http / crypto run at native perf rather than through unenv's no-op
stubs. Updating unenv = compat improvements for free.

Polyfill bundle: **~458 KB raw → ~73 KB zstd-compressed** (embedded in
the binary, decompressed once on startup, ~1 ms cost). Includes
unenv@1.10.0, web-streams-polyfill@4.2.0, and a small Tynd overlay for
fs/http/crypto/process/os routed through the host bridges.

## Web platform (built-ins)

| API                                        | Status | Notes                                  |
|--------------------------------------------|--------|----------------------------------------|
| `fetch` / `Headers` / `Request` / `Response` | OK     | Backed by host `ureq` over native TLS. |
| `URL` / `URLSearchParams`                  | OK     | Pragmatic WHATWG subset.               |
| `FormData` / `Blob` / `File`               | OK     | In-memory.                             |
| `WebSocket`                                | OK     | Backed by host `tungstenite`.          |
| `EventSource`                              | OK     | Built on `fetch`.                      |
| `BroadcastChannel`                         | NO     | No multi-thread runtime in lite. Use IPC events from `@tynd/core/client`. |
| `crypto.getRandomValues` / `randomUUID`    | OK     | UUIDv7 (RFC 9562) by default.          |
| `crypto.subtle.digest/sign/verify/importKey` | PARTIAL | HMAC + SHA-256/384/512 only. Use `@noble/ciphers` for AES, `@noble/curves` for ECDSA. |
| `TextEncoder` / `TextDecoder`              | OK     | UTF-8 only (WHATWG default).           |
| `atob` / `btoa`                            | OK     | Latin-1.                               |
| `structuredClone`                          | OK     | Deep, handles cycles, Date, Map, Set.  |
| `queueMicrotask` / `setImmediate`          | OK     | Mapped to `Promise.resolve().then()`.  |
| `setTimeout` / `setInterval`               | OK     | Backed by Rust timer thread.           |
| `performance.now()`                        | OK     | Monotonic, anchored at runtime start.  |
| `AbortController` / `AbortSignal`          | OK     |                                        |
| `ReadableStream` / `WritableStream` / `TransformStream` | OK | Pragmatic, no full backpressure spec.  |
| `Worker` (Web Workers)                     | NO     | QuickJS is single-threaded. Use `os.workers` from `@tynd/core/client`. |
| `WebAssembly`                              | NO     | rquickjs has no WASM compiler. File an issue if needed. |

## Node built-in modules (`node:*` and bare)

| Module           | Status  | Notes                                                           |
|------------------|---------|-----------------------------------------------------------------|
| `assert` / `assert/strict` | OK | All `strictEqual`, `deepStrictEqual`, `throws`, `rejects`, `match`. |
| `buffer`         | OK      | `Buffer.from/alloc/concat`, hex/base64/utf8/utf16le/latin1, all `read*`/`write*` numeric. |
| `child_process`  | SHIM    | All entry points throw with redirect to `os.process` from `@tynd/core/client`. |
| `console`        | OK      | Routed to host log.                                              |
| `constants`      | OK      | Numeric subset.                                                 |
| `crypto`         | PARTIAL | `createHash/createHmac/randomBytes/randomUUID/randomInt/timingSafeEqual` over SHA-256/384/512. AES/RSA/PBKDF2 throw with `@noble/*` redirect. |
| `dgram`          | NO      | UDP needs a native socket bridge.                                |
| `dns` / `dns/promises` | SHIM | Throws redirect; use IP literals or DoH via fetch.            |
| `domain`         | OK      | Minimal — `create()` returns an EE.                              |
| `events`         | OK      | Full EventEmitter (`on/once/off/emit/...`, `EventEmitter.once/on/listenerCount`). |
| `fs` / `fs/promises` | OK  | `readFile/writeFile/stat/readdir/mkdir/rm/rename/copy/realpath/chmod` sync + async + callback. `createReadStream`/`createWriteStream` chunked. |
| `http` / `https` | PARTIAL | Client (`request/get`) backed by fetch. `createServer` throws with redirect to `os.http_server`. |
| `inspector`      | SHIM    | No-op.                                                           |
| `module`         | OK      | `createRequire` returns the global require.                      |
| `net` / `tls`    | NO      | Raw TCP needs a socket bridge. Use `fetch`/WebSocket/Tynd `os.tcp` if available. |
| `os`             | OK      | `homedir/tmpdir/hostname/cpus/totalmem/freemem/uptime/...` over host `os_info` + `system_stats`. |
| `path` / `path/posix` / `path/win32` | OK | Full Node path semantics.                          |
| `perf_hooks`     | OK      | `performance.now`; `PerformanceObserver` is a no-op stub.        |
| `process`        | OK      | `env/argv/pid/cwd/chdir/exit/platform/arch/versions/nextTick/hrtime/uptime/...`. `dlopen` throws with native-addon redirect. |
| `punycode`       | PARTIAL | Pass-through (no real punycode encoding).                        |
| `querystring`    | OK      | Full `parse/stringify/escape/unescape`.                          |
| `readline`       | PARTIAL | `createInterface` returns an EE stub. No real TTY.               |
| `stream` / `stream/promises` / `stream/web` | OK | Readable/Writable/Transform/Duplex/PassThrough + Web Streams. |
| `string_decoder` | OK      | Backed by `TextDecoder`.                                         |
| `timers` / `timers/promises` | OK | `setTimeout/Interval/Immediate` + Promise wrappers.       |
| `url`            | OK      | Node-style `parse/format/resolve/fileURLToPath/pathToFileURL` over WHATWG URL. |
| `util` / `util/types` | OK | `inspect/promisify/callbackify/format/isDeepStrictEqual/types.*`. |
| `vm`             | PARTIAL | `runInThisContext/runInNewContext` via `eval`. `Script` works.   |
| `worker_threads` | NO      | Use `os.workers` from `@tynd/core/client`.                       |
| `zlib`           | SHIM    | Throws with redirect to `CompressionStream` / `pako` / `@tynd/plugin-zlib`. |
| `node:test`      | PARTIAL | Minimal `test/it/describe/before/after`. No reporter.            |

## Bun runtime APIs

| Bun API                   | Status  | Notes                                                |
|---------------------------|---------|------------------------------------------------------|
| `Bun.env`                 | OK      | Alias for `process.env`.                             |
| `Bun.argv`                | OK      | Alias for `process.argv`.                            |
| `Bun.version`             | OK      | Reports `1.0.0-tynd-lite-shim`.                      |
| `Bun.file(path)`          | OK      | `text/json/arrayBuffer/bytes/blob/stream/exists/writer`, `.size`, `.type`, `.name`. |
| `Bun.write(dest, data)`   | OK      | string / Uint8Array / ArrayBuffer / Blob / Response. |
| `Bun.fileURLToPath` / `pathToFileURL` | OK |                                              |
| `Bun.sleep(ms)`           | OK      |                                                      |
| `Bun.sleepSync(ms)`       | NO      | Best-effort no-op (no sync sleep in QuickJS).        |
| `Bun.nanoseconds()`       | OK      | BigInt.                                              |
| `Bun.gc()`                | OK      | No-op.                                               |
| `Bun.inspect`             | OK      | Routes to `util.inspect`.                            |
| `Bun.deepEquals`          | OK      |                                                      |
| `Bun.deepMatch`           | OK      |                                                      |
| `Bun.escapeHTML`          | OK      |                                                      |
| `Bun.Glob(pattern)`       | PARTIAL | `.match()` works; `.scan()`/`.scanSync()` throw (use `os.fs.scan`). |
| `Bun.password.hash/verify`| PARTIAL | tynd-lite SHA-256-salt scheme. For bcrypt/argon2 use `bcryptjs`/`@noble/hashes`. |
| `Bun.hash` / `Bun.hash.crc32` / `.adler32` | OK | wyhash-flavoured fallback via SHA-256 truncated. |
| `Bun.CryptoHasher`        | OK      | Routes to `crypto.createHash`.                       |
| `Bun.SHA256/384/512`      | OK      | Routes to `crypto.createHash`.                       |
| `Bun.MD5` / `Bun.SHA1`    | SHIM    | Use `@noble/hashes`.                                 |
| `Bun.readableStreamToText/JSON/ArrayBuffer/Bytes/Blob/FormData/Array` | OK | |
| `Bun.concatArrayBuffers`  | OK      |                                                      |
| `Bun.serve(opts)`         | SHIM    | Redirect to `os.http_server` from `@tynd/core/client`. |
| `Bun.listen` / `connect` / `udpSocket` | NO | Raw sockets need a native bridge.            |
| `Bun.spawn` / `spawnSync` | SHIM    | Redirect to `os.process` from `@tynd/core/client`.   |
| `Bun.which`               | PARTIAL | Returns `null` (no PATH lookup).                     |
| `Bun.SQL` / `Bun.sql`     | SHIM    | Redirect to `@tynd/plugin-sql` (cross-platform Rust).|
| `Bun.redis`               | SHIM    | Redirect to `ioredis` (pure-JS).                     |
| `Bun.S3Client` / `Bun.s3` | SHIM    | Redirect to `@aws-sdk/client-s3` (pure-JS) or `@tynd/plugin-s3`. |
| `Bun.dns`                 | SHIM    | Redirect to DoH or platform DNS.                     |
| `Bun.peek`                | OK      | Pass-through.                                        |
| `Bun.plugin`              | NO      | Bundler plugin — define in `tynd.config.ts`.         |
| `Bun.semver`              | PARTIAL | Stub.                                                |

## Native bindings (`.node` files)

**Structurally impossible.** Native Node addons compile against the V8/JSC
ABI; QuickJS exposes a different ABI. `require('foo.node')` and
`process.dlopen` throw a helpful error with the standard plugin redirect:

| npm package(s)              | Tynd alternative                  |
|-----------------------------|-----------------------------------|
| `sharp`, image-codec, pixel-manip | `@tynd/plugin-image` (Rust native) |
| `better-sqlite3`, `sqlite3` | `@tynd/plugin-sql` (drop-in API, SQLite/Postgres/MySQL) |
| `node-pty`                  | `@tynd/plugin-terminal`           |
| `bcrypt` (native), `argon2` | `globalThis.crypto.subtle` (Web Crypto), or `@noble/hashes` (pure JS) |
| `node-canvas`               | Use HTML `<canvas>` from the frontend, expose via IPC |
| `node-sass`                 | `sass` (pure JS, official)        |
| `usb`, `serialport`         | `@tynd/plugin-hid`/serial (planned) — file an issue |

For anything not listed: file an issue at
[github.com/tynd-apps/tynd](https://github.com/tynd-apps/tynd/issues).

## Module loading

- **CommonJS `require()`**: works for all built-in `node:*` modules and
  the Tynd registry. Userland npm packages are statically bundled by
  `tynd build`; the embedded backend never reads `node_modules` at
  runtime.
- **ESM `import`**: works for static imports of the same registry. Same
  bundling rules.
- **`__dirname`/`__filename`/`module`/`exports`**: provided as
  globals; the bundler injects per-file scopes.
- **`import.meta.url`/`import.meta.dir`**: provided where applicable.
- **JSON imports**: handled by the bundler, not at runtime.

## Performance trade-offs

- **No JIT**: rquickjs is an interpreter, ~3-10× slower than V8/JSC for
  CPU-bound code. For hot loops, delegate to native plugins
  (`@tynd/plugin-*`) or move to `tynd-full` (Bun subprocess).
- **Single-threaded**: no `Worker`, no `Atomics`-based parallelism.
  Off-thread compute goes through `os.workers` from
  `@tynd/core/client` (Rust thread-pool with bounded channels).
- **Cold start**: ~50 ms to install all polyfills (decompress + eval).
  See `polyfills/build.rs` for the zstd-19 pre-compression that keeps
  binary size down.

## Testing this

The `tynd-lite` polyfill suite has 24 unit tests covering web standards,
Node modules, Buffer, EventEmitter, querystring, util, streams, assert,
path (POSIX + win32), and Bun.* shim coverage. Run with:

```bash
cd tynd && cargo test -p tynd-lite --lib -- --test-threads=1
```

Add an integration test for any npm package you depend on — the more
coverage, the faster regressions surface.

## Troubleshooting

**"`Cannot find module 'X'`" at runtime**
The embedded backend never reads `node_modules`. Userland packages must
be statically bundled at build time by `tynd build`. If you see this for
a Node builtin (e.g. `dns`, `tls`), it's an unenv mock that throws on
actual use — see the matrix above for the supported alternative.

**"`[unenv] foo.bar is not implemented yet!`"**
A specific unenv stub was reached. Check the matrix; if the surface is
listed PARTIAL/SHIM, follow the redirect. If listed OK, file an issue —
it's a regression vs the documented surface.

**npm package crashes on require**
Most likely a `.node` native addon — see "Native bindings" above for the
plugin redirect. Otherwise run with `TYND_LOG=trace` to surface the
exact module path that failed to resolve.

**Bundle size grew vs. previous Tynd version**
Sprint 1.0 swapped 15 hand-rolled polyfills (~212 KB raw, 43 KB zstd)
for the unenv pipeline (~458 KB raw, 73 KB zstd) — a one-time +30 KB
binary cost in exchange for ~95% npm ecosystem compat (vs ~85% with the
hand-rolled polyfills) and zero ongoing polyfill maintenance.

## Future work

- **`pako`-backed zlib**: pure-JS gzip/deflate so `node:zlib` is fully
  functional without a Rust bridge.
- **DoH-backed `dns`**: implement `dns.resolve` via a hard-coded DoH
  endpoint so DNS works for sandboxed builds.
- **Track unenv `main`**: opt into the `unenv-nightly` channel for
  earlier compat fixes, with the official release as the stable line.
