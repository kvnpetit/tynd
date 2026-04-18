# 🧬 Runtimes — `lite` vs `full`

Tynd ships **two backend runtimes**. Same TypeScript API, same frontend — one line to switch.

```ts
// tynd.config.ts
export default {
  runtime: "full",  // or "lite"
}
```

## What each runtime is

- **`full`** spawns a **Bun subprocess**. You get the entire Bun + Node.js + Web-API environment: `fetch`, `WebSocket`, `Bun.*`, `node:*` imports, native npm packages, JSC JIT. Binary overhead: ~37 MB (Bun packed + zstd-compressed at build time).
- **`lite`** embeds **QuickJS** inside the Rust host. QuickJS only provides the **ES2023 language** (Promises, classes, Proxy, BigInt, Maps/Sets, etc.). Tynd adds a **strict Web-standards polyfill layer** on top — nothing Node-specific, nothing Bun-specific. Binary target: ~6 MB, single self-contained executable.

Both runtimes expose the same **Tynd OS APIs** (`fs`, `http`, `websocket`, `sql`, `process`, `store`, `compute`, `workers`, `terminal`, `sidecar`, `singleInstance`, `dialog`, `clipboard`, `shell`, `notification`, `tray`, `tyndWindow`, `menu`) via `@tynd/core/client`. Those are Rust-backed and work identically everywhere. Use them for any OS access.

---

## 🚩 Notable absences in lite — read before shipping

Most code written against Web standards runs unchanged on both runtimes. These are the **behavioral differences** most likely to bite when you port code from `full` to `lite` (or write lite-first and hit a wall). They're the cost of a 6 MB binary — no surprises once you know them:

| What | Behavior in lite | Fix |
|---|---|---|
| **`Response.clone()` / `Request.clone()`** | Throws `"not supported in lite runtime"`. | Consume the body once, stash the bytes, rebuild as needed. |
| **HTTP/2, HTTP/3** | Not supported — lite's fetch bridges to Rust `ureq` (HTTP/1.1 only). | Upgrade your server to serve HTTP/1.1 alongside, or switch to `full` (Bun supports h2). |
| **`WritableStream` / `TransformStream`** | Not implemented — only `ReadableStream` (for fetch body) is shipped. | [`web-streams-polyfill`](https://github.com/MattiasBuelens/web-streams-polyfill) (~15 KB) or switch to `full`. |
| **Streaming upload with backpressure** | A `ReadableStream` body is **drained into memory** before the request is sent — no server-pressure-aware streaming. | For large uploads, chunk manually via `http.request` + multiple calls, or switch to `full`. |
| **`structuredClone` with `{ transfer }`** | Throws — transfer lists unsupported (value is cloned, not transferred). | Drop the transfer list; or use `workers` message passing. |
| **`CompressionStream` / `DecompressionStream`** | Absent. | [`fflate`](https://github.com/101arrowz/fflate) for gzip/deflate. |
| **`crypto.subtle.encrypt` / `decrypt`, asym sign/verify** | Only HMAC + digest; AES / RSA / ECDSA throw. | [`@noble/ciphers`](https://github.com/paulmillr/noble-ciphers) + [`@noble/curves`](https://github.com/paulmillr/noble-curves). |
| **Dynamic `import(path)` at runtime** | Not supported — lite backend is a single eval'd bundle. | Bundle all modules at build time. |
| **`SharedArrayBuffer` / `Atomics` / `WeakRef` / `FinalizationRegistry`** | QuickJS limitation. | Use `workers` with JSON message passing. |
| **Full `Intl.*` locale data** | QuickJS ships a stub; `DateTimeFormat`, `Collator`, `Segmenter`, `RelativeTimeFormat` are unavailable or English-only. | `date-fns` / `dayjs` / `i18next`. |

All other Web APIs listed in the next section behave identically on both runtimes — same input, same output, same error shape.

---

## What lite exposes (Web standards only)

This is the entire JS-level surface available in a lite backend. Everything in the list works the same way on `full`.

| Web API | Notes |
|---|---|
| `fetch` + `Request` / `Response` / `Headers` / `ReadableStream` (body) | HTTP/1.1 only. Streaming *download* bodies. Upload drained to memory first. AbortSignal, Blob / URLSearchParams / FormData bodies, multipart encoding. `Response.clone()` throws. |
| `WebSocket` | `addEventListener` + `on*` properties, text + binary frames |
| `EventSource` | Built on top of fetch streaming |
| `AbortController` / `AbortSignal` / `AbortSignal.timeout` | — |
| `crypto.getRandomValues` / `crypto.randomUUID` | OS CSPRNG |
| `crypto.subtle.digest` (SHA-256 / SHA-384 / SHA-512) | Rust `sha2` bridge |
| `crypto.subtle.sign` / `verify` / `importKey` — **HMAC only** | For AES / RSA / ECDSA see [ALTERNATIVES.md](ALTERNATIVES.md) |
| `TextEncoder` / `TextDecoder` | UTF-8 with streaming |
| `URL` / `URLSearchParams` | Pragmatic WHATWG subset (no IDN / punycode) |
| `Blob` / `File` / `FormData` | — |
| `atob` / `btoa` | — |
| `structuredClone` | Date / Map / Set / ArrayBuffer / typed arrays / cycles |
| `Promise.withResolvers` | — |
| `performance.now()` | Monotonic Rust `Instant` |
| `console.log/info/warn/error/debug` | — |
| `setTimeout` / `setInterval` / `queueMicrotask` | — |

**That's it.** No `Buffer`, no `process.*`, no `Bun.*`, no `Deno.*`, no `node:*` imports, no raw sockets, no HTTP server, no JIT. Lite is a JS runtime, not a Node/Bun environment.

---

## How to do X in lite

Anything that reaches past the Web surface goes through a Tynd OS API. These are Rust-backed, identical in lite and full, and the same imports work on both runtimes.

| Task | What to write |
|---|---|
| Read / write a file | `import { fs } from "@tynd/core/client"` → `await fs.readText(path)` |
| Watch a file or directory | `fs.watch` is not exposed yet — switch to `full` or poll with `fs.stat` |
| Hash or RNG bytes | `import { compute } from "@tynd/core/client"` → `compute.hash(bytes, { algo: "sha256" })` / `compute.randomBytes(32)` |
| Spawn a subprocess | `import { process } from "@tynd/core/client"` → `process.exec(cmd, { args })` |
| Bundled binary (ffmpeg, etc.) | `import { sidecar, process } from "@tynd/core/client"` → `process.exec(await sidecar.path("ffmpeg"))` |
| Read env vars | `import { os } from "@tynd/core/client"` → `os.env("API_KEY")` |
| Persistent key/value | `import { createStore } from "@tynd/core/client"` |
| Embedded SQLite | `import { sql } from "@tynd/core/client"` |
| WebSocket client | `new WebSocket(url)` (Web standard) or `websocket.connect` (Tynd API, with more hooks) |
| HTTP client | `fetch(url, opts)` (Web standard) or `http.request` (Tynd API, with upload/download progress) |
| File / folder dialog | `import { dialog } from "@tynd/core/client"` |
| Tray + menu | `import { tray, menu } from "@tynd/core/client"` |
| Native notification | `import { notification } from "@tynd/core/client"` |
| PTY terminal | `import { terminal } from "@tynd/core/client"` |
| Isolated worker | `import { workers } from "@tynd/core/client"` |
| Single-instance lock | `import { singleInstance } from "@tynd/core/client"` |

---

## What full can do that lite can't

| Capability | `full` | `lite` | Workaround in lite |
|---|---|---|---|
| JIT performance for CPU-bound JS | ✓ (JSC) | ✗ interpreter | Move hot paths to Rust via `compute`, `workers`, or a sidecar. |
| npm packages with **native bindings** (sharp, better-sqlite3, canvas, bcrypt-native, rocksdb) | ✓ | ✗ | Pure-JS alternative ([ALTERNATIVES.md](ALTERNATIVES.md)) or switch to full. |
| `Bun.*`, `Deno.*`, `node:*` imports | ✓ | ✗ intentionally | Use the Tynd OS APIs above or a pure-JS lib. |
| `Buffer` (Node global) | ✓ | ✗ | Use `Uint8Array` — every Tynd OS API already accepts it. |
| `process.env` / `process.exit` / `process.on("SIGINT")` | ✓ | ✗ | `os.env()`, app-lifecycle handlers in `@tynd/core`. |
| Raw TCP / UDP sockets, HTTP server, DNS lookup, gzip, `fs.watch` | ✓ | ✗ | These are not exposed as JS APIs in lite; they'll likely land as Tynd OS APIs. For now, switch to full. |
| HTTP/2, HTTP/3 client | ✓ | ✗ HTTP/1.1 only | Server must serve HTTP/1.1 fallback, or switch to full. |
| `Response.clone()` / `Request.clone()` | ✓ | ✗ throws | Consume body once, cache bytes, rebuild as needed. |
| `WritableStream` / `TransformStream` / `CompressionStream` | ✓ | ✗ | [`web-streams-polyfill`](https://github.com/MattiasBuelens/web-streams-polyfill), [`fflate`](https://github.com/101arrowz/fflate) for compression. |
| Streaming upload with real backpressure | ✓ | ✗ drained first | Chunk manually via `http.request`, or switch to full. |
| `crypto.subtle.encrypt` / `decrypt` (AES-GCM, RSA, ECDSA) | ✓ | ✗ | See [ALTERNATIVES.md](ALTERNATIVES.md) — `@noble/ciphers`, `@noble/curves`. |
| Password hashing (argon2id, bcrypt) | ✓ | ✗ | `@noble/hashes` (pure-JS argon2) or switch to full. |
| PBKDF2 / HKDF / scrypt key derivation | ✓ | ✗ | `@noble/hashes`. |
| Dynamic `import()` at runtime | ✓ | ✗ | Lite backend is a single eval'd bundle. Bundle every module at build time. |
| `SharedArrayBuffer` / `Atomics` / `WeakRef` / `FinalizationRegistry` | ✓ | ✗ | rquickjs limits — use `workers` with message passing instead. |
| Chrome DevTools inspector | ✓ | ✗ | Debug under `full` during development. |
| Full `Intl.*` (DateTimeFormat, Collator, Segmenter, RelativeTimeFormat) | ✓ ICU bundled | ✗ | `date-fns` / `dayjs` / `i18next`. |

---

## When to pick which

**Default to `lite`.** Most desktop apps only need the Web-standard surface plus the Tynd OS APIs, and the 1.5 MB lighter binary + zero Bun download is a real UX win.

**Pick `full` when:**
- Your dependency graph contains an npm with native bindings you can't replace.
- You have a clear JS hot-path profiled as the bottleneck.
- You depend on specific Bun / Node APIs not exposed as Web standards or Tynd OS APIs.
- You need `Intl.*` locale formatting and can't ship `date-fns`.

The `tynd.config.ts` switch is one line; the same source code runs on both runtimes as long as it stays on the Tynd OS APIs + Web standards.
