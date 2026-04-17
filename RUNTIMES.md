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
| `import("fs")` / `import("path")` / `import("os")` | ✗ | ✓ |
| `Intl` (date/number formatting) | ✗ | ✓ |
| `Worker` / threads | ✗ | ✓ |
| npm packages with native bindings | ✗ | ✓ |

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

Lite has no persistence. Use `full` with `bun:sqlite`, or keep state in-memory with a `Map`.

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

- Your backend does validation, routing, data transforms — no OS access needed
- You want a self-contained binary with no external runtime
- You need high concurrent call throughput

### Use `full` when

- Your backend reads or writes files
- You need a local database (SQLite)
- You make HTTP requests or use WebSockets from the backend
- You use npm packages with native bindings
- You need `Intl`, `crypto`, `Buffer`, or the full Node/Bun API surface
