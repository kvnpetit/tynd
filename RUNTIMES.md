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

> **Start with `full`.** It gives you the complete Bun runtime — every npm package, file system, SQLite, `fetch`. Zero restrictions.

Switch to `lite` only if:
- You want a **tiny binary** (~5 MB, no runtime to ship)
- Your backend doesn't need FS, network, or native packages

---

## What each mode supports

Two axes matter: the **JS runtime** features (what the VM provides) and the **Tynd OS APIs** (`process`, `fs`, `os`, `store`, `path`, …) which are implemented in Rust and available to **both** runtimes identically.

### JS runtime surface

| Capability | `lite` | `full` |
|-----------|--------|--------|
| ES2023 language (async/await, Proxy, BigInt, generators…) | ✓ | ✓ |
| `console.log/warn/error` | ✓ | ✓ |
| `setTimeout` / `setInterval` | ✓ | ✓ |
| Pure-JS npm packages | ✓ | ✓ |
| `URL` / `URLSearchParams` | ✗ | ✓ |
| `TextEncoder` / `TextDecoder` | ✗ | ✓ |
| `atob` / `btoa` | ✗ | ✓ |
| `structuredClone` | ✗ | ✓ |
| `Buffer` | ✗ | ✓ |
| `crypto` (hashing, random, subtle) | ✗ | ✓ |
| `fetch` / `WebSocket` | ✗ | ✓ |
| `Bun.file()` / `Bun.write()` | ✗ | ✓ |
| `bun:sqlite` | ✗ | ✓ |
| `import("fs")` / `import("path")` / `import("os")` (Node modules) | ✗ | ✓ |
| `Intl` (date/number formatting) | ✗ | ✓ |
| `Worker` / threads | ✗ | ✓ |
| npm packages with native bindings | ✗ | ✓ |

### Tynd OS APIs (identical in both runtimes)

Routed through the Rust host, so lite and full share the exact same surface:

| API | Methods |
|---|---|
| `process` | `exec`, `execShell` (run OS commands, capture stdout/stderr/code) |
| `fs` | `readText`, `writeText`, `exists`, `stat`, `readDir`, `mkdir`, `remove`, `rename`, `copy` |
| `path` | `join`, `dirname`, `basename`, `extname`, `sep` (pure TS) |
| `os` | `info`, `homeDir`, `tmpDir`, `configDir`, `dataDir`, `cacheDir`, `exePath`, `cwd`, `env` |
| `store` | `createStore(ns)` → `get`, `set`, `delete`, `clear`, `keys` (JSON-backed k/v) |
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
| `tweetnacl` | ⚠️ | Deterministic ops ✓ — key generation needs manual PRNG |
| `nanoid` | ⚠️ | Works with `Math.random`-based generator |
| `uuid` v9+ | ✗ | Requires `crypto.getRandomValues` |
| `axios` | ✗ | Requires network stack |
| `sharp` | ✗ | Native binding |
| `better-sqlite3` | ✗ | Native binding |
| `bcrypt` | ✗ | Native binding |
| `node-fetch` | ✗ | Requires Node http module |

---

## Lite mode — workarounds for missing APIs

### Crypto

| Use case | Library |
|----------|---------|
| SHA-256, SHA-512, BLAKE2 | [`@noble/hashes`](https://github.com/paulmillr/noble-hashes) |
| AES, ChaCha20 | [`@noble/ciphers`](https://github.com/paulmillr/noble-ciphers) |
| Ed25519, secp256k1 | [`@noble/curves`](https://github.com/paulmillr/noble-curves) |

> Key generation requires a PRNG. `Math.random()` is not cryptographically secure — use `full` for security-sensitive key generation.

### UUID

```typescript
// Non-security UUID — no crypto needed
function uuidv4(): string {
  return "xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx".replace(/[xy]/g, c => {
    const r = Math.random() * 16 | 0
    return (c === "x" ? r : (r & 0x3 | 0x8)).toString(16)
  })
}
```

### Date / number formatting

Use [`date-fns`](https://date-fns.org/) or [`dayjs`](https://day.js.org/) instead of `Intl`.

### Local storage

Use the built-in `store` API (JSON-backed key/value, works in both runtimes):

```ts
import { createStore } from "@tynd/core/client"
const prefs = createStore("com.example.myapp")
await prefs.set("theme", "dark")
const theme = await prefs.get<string>("theme")
```

For relational data in lite, serialize through `fs.writeText` + JSON. For SQL-heavy apps, use `full` with `bun:sqlite`.

---

## Performance

Measured on Windows 11, Bun 1.3.11, release binaries. Numbers are indicative — real workloads vary.

| Benchmark | `lite` | `full` | Winner |
|-----------|--------|--------|--------|
| Startup — first call | 0.5 ms | 0.7 ms | lite +40% |
| Startup — load 100 items | 0.5 ms | 1.7 ms | lite +240% |
| IPC floor (warmed) | 0.3 ms | 0.3 ms | tie |
| Filter+sort 200 items | 0.8 ms | 0.3 ms | full +167% |
| Filter+sort 2 000 items | 3.8 ms | 0.6 ms | full +533% |
| 100 concurrent calls | 0.031 ms/call | 0.069 ms/call | lite +123% |
| Sustained call rate | 4 200/s | 3 200/s | lite +31% |
| Send+receive 100 KB | 8.6 ms | 3.6 ms | full +139% |
| Read file — 1 MB | N/A | 0.6 ms | full only |
| SQLite aggregate | N/A | 1.6 ms | full only |

Both modes feel instant for typical user interactions — all values are well under 10ms.

---


## When to choose each

### Use `lite` when

- You want a tiny self-contained binary (~5 MB, no runtime to ship)
- OS access stays within the Tynd APIs (`process`, `fs`, `store`, `dialog`, …)
- You need high concurrent call throughput

### Use `full` when

- You need SQL (`bun:sqlite`) or HTTP/WebSockets from JS
- You use npm packages with native bindings
- You need `Intl`, `crypto`, `Buffer`, `fetch`, or the full Node/Bun API surface directly from JS
