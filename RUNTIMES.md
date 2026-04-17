# 🧬 Runtimes — `lite` vs `full`

Tynd ships **two backend runtimes**. Same TypeScript API, same frontend — one line to switch.

```ts
// tynd.config.ts
export default {
  runtime: "full",  // or "lite"
}
```

---

## ⚡ Quick decision

> **Start with `lite`.** Most apps are covered: the Tynd OS APIs (`http`, `websocket`, `sql`, `fs`, `process`, `store`, `compute`, `workers`, `terminal`, `sidecar`, `crashReporter`, `singleInstance`, `dialog`, `clipboard`, `shell`, `notification`, `tray`, `tyndWindow`) are Rust-backed and work identically in both runtimes. Lite ships a ~5 MB binary with no external runtime.

Switch to `full` only if you need:
- Direct JS access to `fetch` / `Intl` / `Buffer` (lite doesn't expose these to JS — Tynd's equivalents cover most cases but not every corner).
- npm packages with **native bindings** (sharp, better-sqlite3, bcrypt, canvas).
- Large CPU-bound workloads that benefit from JSC's JIT (QuickJS is an interpreter).

---

## What each mode supports

Two axes matter: the **JS runtime** features (what the VM provides) and the **Tynd OS APIs** (`process`, `fs`, `os`, `store`, `path`, …) which are implemented in Rust and available to **both** runtimes identically.

### JS runtime surface

Where lite is ✗, check the **Tynd equivalent** column — most gaps are closed by an OS API that works identically in both runtimes.

| Capability | `lite` | `full` | Tynd equivalent |
|-----------|--------|--------|-----------------|
| ES2023 (async/await, Proxy, BigInt, generators…) | ✓ | ✓ | — |
| `console.log/warn/error` | ✓ | ✓ | — |
| `setTimeout` / `setInterval` | ✓ | ✓ | — |
| Pure-JS npm packages | ✓ | ✓ | — |
| `URL` / `URLSearchParams` | ✗ | ✓ | Pure-JS polyfills or use `http` API |
| `TextEncoder` / `TextDecoder` | ✗ | ✓ | `fs` + `compute` accept `Uint8Array` directly |
| `atob` / `btoa` | ✗ | ✓ | `@tynd/core/client` base64 helpers (internal) |
| `structuredClone` | ✗ | ✓ | `JSON.parse(JSON.stringify(x))` |
| `Buffer` | ✗ | ✓ | `Uint8Array` (Tynd APIs use `Uint8Array` natively) |
| `crypto.subtle` hashing | ✗ | ✓ | `compute.hash` (blake3 / sha256 / sha512) |
| `crypto.getRandomValues` | ✗ | ✓ | `compute.randomBytes(n)` (Rust `OsRng`) |
| `fetch` / `WebSocket` | ✗ | ✓ | `http` API for HTTP; `websocket` API for WS |
| `Bun.file()` / `Bun.write()` | ✗ | ✓ | `fs.readText` / `fs.writeText` / binary variants |
| `bun:sqlite` | ✗ | ✓ | `sql` API (embedded SQLite via rusqlite) |
| `import("fs")` / `import("child_process")` etc. | ✗ | ✓ | `fs` + `process` OS APIs |
| `Intl` | ✗ | ✓ | Pure-JS libs like `date-fns`, `dayjs`, `i18next` |
| `Worker` / threads | ✗ | ✓ | `workers` API (isolated QuickJS in lite, `Bun.Worker` in full) |
| Compression | ✗ | ✓ | `compute.compress` / `decompress` (zstd) |
| Spawn child processes | ✗ | ✓ | `process.exec` / `execShell` |
| Run embedded binaries | ✗ | ✓ | `sidecar.path` + `process.exec` |
| Embedded terminal | ✗ | ✓ | `terminal.spawn` (portable-pty) |
| Crash reports | ✗ | ✓ | `crashReporter.enable` (Rust panic hook -> file) |
| Single-instance lock | ✗ | ✓ | `singleInstance.acquire(id)` |
| npm packages with native bindings | ✗ | ✓ | **full only** |

### Tynd OS APIs (identical in both runtimes)

Routed through the Rust host, so lite and full share the exact same surface. APIs marked **\*** stream raw bytes through the dedicated `tynd-bin://` custom protocol (no base64 on the wire) for multi-MB payloads; the rest use JSON IPC.

| API | Methods |
|---|---|
| `process` | `exec`, `execShell` (run OS commands, capture stdout/stderr/code) |
| `fs` | `readText`, `writeText`, `readBinary`\*, `writeBinary`\*, `exists`, `stat`, `readDir`, `mkdir`, `remove`, `rename`, `copy` |
| `path` | `join`, `dirname`, `basename`, `extname`, `sep` (pure TS) |
| `os` | `info`, `homeDir`, `tmpDir`, `configDir`, `dataDir`, `cacheDir`, `exePath`, `cwd`, `env` |
| `store` | `createStore(ns)` -> `get`, `set`, `delete`, `clear`, `keys` (JSON-backed k/v) |
| `http` | `get`, `getJson`, `getBinary`, `post`, `request`, `download` (TLS via rustls) |
| `websocket` | `connect(url)` -> `{ send, ping, close, onOpen, onMessage, onClose, onError }` — tungstenite + rustls |
| `sql` | `open(path)` -> `{ exec, query, queryOne, close }` — bundled SQLite via rusqlite |
| `sidecar` | `path(name)`, `list()` — binaries bundled at build time, extracted at startup |
| `terminal` | `spawn({ shell, cols, rows, cwd, env })` -> PTY handle with `write`, `resize`, `kill`, `onData`, `onExit` |
| `compute` | `hash(data, { algo })`\*, `compress`\* / `decompress`\* (zstd), `randomBytes(n)` (CSPRNG) |
| `workers` | `spawn(fn)` -> `{ run, terminate }`; `parallel.map(items, fn, { concurrency })`. Lite: isolated QuickJS on thread. Full: wraps `Bun.Worker`. |
| `singleInstance` | `acquire(id)` -> `{ acquired, already }` — cross-OS exclusive lock (named pipe / abstract socket) held for process lifetime |
| `crashReporter` | `enable(appId)` installs a panic hook that writes `crash-<unix-nanos>.log` files under `data_dir/<appId>/crashes/`; `listCrashes()` returns the paths |
| `dialog` | `openFile`, `openFiles`, `saveFile`, `message`, `confirm` |
| `clipboard` | `readText`, `writeText` |
| `shell` | `openExternal`, `openPath` |
| `notification` | `send` |
| `tray` | click/menu subscriptions |
| `tyndWindow` | title/size/visibility/fullscreen/always-on-top/decorations |

---

## npm package compatibility

| Package | `lite` | Notes |
|---------|--------|-------|
| `zod` | ✓ | Pure-JS validation |
| `valibot` | ✓ | Pure-JS validation |
| `date-fns` | ✓ | Pure-JS date utilities |
| `dayjs` | ✓ | Pure-JS date library |
| `lodash-es` | ✓ | Pure-JS utilities |
| `fflate` | ✓ | Pure-JS compression |
| `@noble/hashes` | ✓ | SHA-256, BLAKE2, etc. — no random needed |
| `tweetnacl` | ✓ | Seed with `compute.randomBytes` for key generation |
| `nanoid` | ✓ | Feed `compute.randomBytes` or use the `Math.random`-based generator |
| `uuid` v9+ | ⚠️ | Provide a getRandomValues shim backed by `compute.randomBytes` |
| `axios` | ✗ | Requires network stack |
| `sharp` | ✗ | Native binding |
| `better-sqlite3` | ✗ | Native binding |
| `bcrypt` | ✗ | Native binding |
| `node-fetch` | ✗ | Requires Node http module |

---

## Lite mode — workarounds for missing APIs

### Crypto

| Use case | Recommendation |
|----------|---------|
| SHA-256, SHA-512, BLAKE3 hashing | **`compute.hash(data, { algo })`** — Rust-native, ~3x faster than JS libs |
| Zstd compress / decompress | **`compute.compress` / `decompress`** — Rust-native |
| HMAC, AES, ChaCha20 | [`@noble/ciphers`](https://github.com/paulmillr/noble-ciphers) (pure JS, works in lite) |
| Ed25519, secp256k1 | [`@noble/curves`](https://github.com/paulmillr/noble-curves) (pure JS) |
| Secure random (CSPRNG) | **`compute.randomBytes(n)`** — Rust `OsRng`, works in both runtimes |

### UUID

```typescript
import { compute } from "@tynd/core/client"

// Cryptographically-random UUID v4, works in both runtimes
async function uuidv4(): Promise<string> {
  const b = await compute.randomBytes(16)
  b[6] = (b[6] & 0x0f) | 0x40
  b[8] = (b[8] & 0x3f) | 0x80
  const hex = Array.from(b, (x) => x.toString(16).padStart(2, "0")).join("")
  return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`
}
```

### Date / number formatting

Use [`date-fns`](https://date-fns.org/) or [`dayjs`](https://day.js.org/) instead of `Intl`.

### File reads / writes

Use the **`fs` API** — same surface in lite and full (no `import("fs")` needed, no `Bun.file` needed):

```ts
import { fs } from "@tynd/core/client"

const text = await fs.readText("config.json")
await fs.writeText("state.json", JSON.stringify(state), { createDirs: true })
const bytes = await fs.readBinary("image.png")
```

### HTTP

Use the **`http` API** — replaces `fetch`, with upload + download progress events:

```ts
import { http } from "@tynd/core/client"

const { body } = await http.getJson<Repo[]>("https://api.github.com/users/kvnpetit/repos")
await http.download(url, "./downloads/ffmpeg.zip", {
  onProgress: ({ loaded, total }) => console.log(total ? `${(loaded / total) * 100}%` : loaded),
})
```

### Spawning processes / running sidecar binaries

```ts
import { process, sidecar } from "@tynd/core/client"

const { stdout } = await process.exec("git", { args: ["status"] })
const ffmpeg = await sidecar.path("ffmpeg.exe")
await process.exec(ffmpeg, { args: ["-i", input, output] })
```

### Local storage

Use the built-in `store` API (JSON-backed key/value, works in both runtimes):

```ts
import { createStore } from "@tynd/core/client"
const prefs = createStore("com.example.myapp")
await prefs.set("theme", "dark")
const theme = await prefs.get<string>("theme")
```

For relational data, use the **`sql` API** — bundled SQLite, identical in lite and full:

```ts
import { sql } from "@tynd/core/client"
const db = await sql.open("./data.db")
await db.exec("CREATE TABLE IF NOT EXISTS t(k TEXT PRIMARY KEY, v TEXT)")
await db.exec("INSERT INTO t VALUES (?1, ?2)", ["theme", "dark"])
```

### WebSocket

Use the **`websocket` API** — full-duplex client, works in both runtimes:

```ts
import { websocket } from "@tynd/core/client"
const ws = await websocket.connect("wss://echo.websocket.events")
ws.onMessage((m) => m.kind === "text" && console.log(m.data))
await ws.send("hello")
```

---

## Performance — expected shape

No measurements checked into the repo. The shape below is derived from the architecture; treat it as **qualitative guidance**, not a benchmark.

| Workload | Expected winner | Why |
|---|---|---|
| Cold start, first IPC call | **lite** | No subprocess, no Bun boot; QuickJS runs in-process |
| Sustained simple IPC calls | **lite** | In-process; full pays stdin/stdout JSON round-trip |
| Concurrent IPC throughput | **lite** | Shorter path, no subprocess scheduling |
| CPU-bound JS (filter/sort/parse in JS) | **full** | JSC JIT vs QuickJS interpreter — gap grows with data size |
| Large payload transfer (100 KB+) | **full** | Bigger OS pipes, native serialisation in JSC |
| `fs` / `http` / `websocket` / `sql` / `compute` / `process` / `workers` | **tie** | Same Rust code runs on both — API name hits the same `os_call` dispatch |
| Raw JS-level `bun:sqlite` (no IPC) | **full only** | Lite exposes SQLite through the `sql` API, which round-trips via IPC |
| Worker pool (`parallel.map`) | **tie on I/O**, **full on CPU** | Same worker spawn path; JSC JIT wins inside hot loops |

Both feel instant for typical user interactions.

---


## When to choose each

### Use `lite` when

- You want the smallest self-contained binary (~5 MB) — no Bun runtime to ship
- All your OS needs fit the Tynd APIs (which now cover `fs`, `http`, `websocket`, `sql`, `process`, `store`, `compute`, `workers`, `terminal`, `sidecar`, `crashReporter`, `singleInstance`, `dialog`, `clipboard`, `shell`, `notification`, `tray`, `tyndWindow`)
- You need high concurrent-call throughput / low startup latency
- You're comfortable with pure-JS npm packages only (no native bindings)

### Use `full` when

- You need a **JS-level** `fetch` / `Intl` / `Buffer` — not a Tynd wrapper
- You use npm packages with **native bindings** (sharp, better-sqlite3, bcrypt, canvas)
- You have CPU-bound JS hot paths that benefit from JSC's JIT

### Rule of thumb

**Start with lite. Switch to full the first time you hit an API Tynd doesn't expose.** The `tynd.config.ts` switch is a one-line edit — no code refactor needed as long as you stayed on the Tynd OS APIs.
