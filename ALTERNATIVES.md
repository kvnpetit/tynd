# 🔁 Alternatives — pure-JS libraries for things lite doesn't polyfill

> **Context first:** see [`RUNTIMES.md`](./RUNTIMES.md) for the full `lite` vs `full` parity table — which JS globals exist, which Node/Bun APIs are absent, and which Tynd OS APIs fill the same role natively on both runtimes. This file is the companion catalog: **when you need something `lite` doesn't expose, here's the pure-JS library that runs on both runtimes.**

Lite is deliberately restricted to Web standards (WHATWG + W3C + TC39). Everything below is a feature `full` has natively (via Bun) that `lite` doesn't — and for which a **pure-JS npm package** already does the job. Ship the lib in your app and the same code runs on both runtimes.

Each row has been checked for: (1) pure JS (no native bindings), (2) reasonable bundle size, (3) active maintenance.

**When in doubt:** if your dep ships native bindings (`sharp`, `better-sqlite3`, `canvas`, `bcrypt` native) or uses `bun:ffi` / dynamic `import(path)`, switch to `runtime: "full"` in `tynd.config.ts` — see the [bottom of this doc](#when-to-just-switch-to-full).

## Crypto

| You'd reach for | Recommended pure-JS lib | Notes |
|---|---|---|
| `crypto.subtle.encrypt` / `decrypt` (AES-GCM, AES-CBC, ChaCha20) | **[`@noble/ciphers`](https://github.com/paulmillr/noble-ciphers)** | AES-GCM, AES-CBC, AES-CTR, ChaCha20-Poly1305. ~10 KB, audited. |
| `crypto.subtle` RSA-PSS / RSA-OAEP / ECDSA / Ed25519 | **[`@noble/curves`](https://github.com/paulmillr/noble-curves)** + **[`@noble/hashes`](https://github.com/paulmillr/noble-hashes)** | Covers secp256k1, P-256, P-384, Ed25519, X25519, RSA. Audited. |
| `Bun.password` (argon2id, bcrypt) | **[`@noble/hashes/argon2`](https://github.com/paulmillr/noble-hashes)** for argon2id<br>**[`bcryptjs`](https://github.com/dcodeIO/bcrypt.js)** for bcrypt | Pure JS argon2 is ~10x slower than native; acceptable for auth (runs once on login, not per request). |
| `crypto.pbkdf2` / `hkdf` / `scrypt` key derivation | **[`@noble/hashes`](https://github.com/paulmillr/noble-hashes)** (`pbkdf2`, `hkdf`, `scrypt` modules) | All three algos, audited, ~5 KB. |
| `Bun.CryptoHasher` streaming | `@noble/hashes` exposes streaming `.update().digest()` for every algo. |

## Compression

| You'd reach for | Recommended | Notes |
|---|---|---|
| `node:zlib` gzip / deflate / raw-deflate | **[`fflate`](https://github.com/101arrowz/fflate)** | Fastest pure-JS gzip implementation. Sync + async + streaming. ~8 KB. |
| `CompressionStream` / `DecompressionStream` | **[`fflate`](https://github.com/101arrowz/fflate)** + `web-streams-polyfill` | Wrap fflate in a `TransformStream` from `web-streams-polyfill` to get the spec API. |
| `node:zlib` brotli | **[`brotli-dec-wasm`](https://github.com/httptoolkit/brotli-wasm)** or **[`brotli`](https://github.com/foliojs/brotli.js)** | WASM-based, ~200 KB. |
| Zstd compression | **`compute.compress` isn't exposed publicly** — zstd is an internal TYNDPKG detail. Use `fflate` + gzip for network/file compression. |

## Internationalization

| You'd reach for | Recommended | Notes |
|---|---|---|
| `Intl.DateTimeFormat` | **[`date-fns`](https://date-fns.org/)** or **[`dayjs`](https://day.js.org/)** | Both have locale packs; tree-shakeable. |
| `Intl.NumberFormat` | **[`numbro`](https://numbrojs.com/)** | Locale-aware number / currency formatting. |
| `Intl.RelativeTimeFormat` | `date-fns/formatDistance` / `dayjs/plugin/relativeTime` | — |
| `Intl.Segmenter` (word/sentence breaking) | **[`graphemer`](https://github.com/flmnt/graphemer)** for graphemes<br>**[`tokenizr`](https://github.com/rse/tokenizr)** for custom | No pure-JS covers full ICU, these cover the common cases. |
| `Intl.Collator` (locale-aware sort) | **[`@intl-js/collator`](https://formatjs.io/docs/polyfills/intl-collator/)** from FormatJS | — |

## HTTP / networking (when the Tynd OS API isn't enough)

| You'd reach for | Recommended | Notes |
|---|---|---|
| Node's HTTP client ergonomics on top of `fetch` | **[`ofetch`](https://github.com/unjs/ofetch)** or **[`ky`](https://github.com/sindresorhus/ky)** | Both wrap `fetch`; work fine against our polyfill. |
| GraphQL | **[`graphql-request`](https://github.com/jasonkuhrt/graphql-request)** | Works on `fetch`. |
| JSON Web Tokens | **[`jose`](https://github.com/panva/jose)** | HS256 / HS384 / HS512 work against our HMAC polyfill. RS256 needs `@noble/curves` or switch to full. |
| OAuth flows | **[`oauth4webapi`](https://github.com/panva/oauth4webapi)** | PKCE + all grant types, pure JS. |

## Data + streaming

| You'd reach for | Recommended | Notes |
|---|---|---|
| `WritableStream` / `TransformStream` | **[`web-streams-polyfill`](https://github.com/MattiasBuelens/web-streams-polyfill)** | Full spec polyfill, ~15 KB. |
| `Response.clone` / `Request.clone` | Same polyfill above. |
| Multipart / form-data parsing (server-side) | **[`@mjackson/multipart-parser`](https://github.com/mjackson/remix)** | Pure JS. |
| BigInt JSON | **[`json-bigint`](https://github.com/sidorares/json-bigint)** | `JSON.parse` / `stringify` with BigInt support. |
| MessagePack | **[`@msgpack/msgpack`](https://github.com/msgpack/msgpack-javascript)** | Pure JS, tree-shakeable. |
| CBOR | **[`cbor-x`](https://github.com/kriszyp/cbor-x)** | Pure JS + optional WASM decoder. |

## File format parsing

| You'd reach for | Recommended |
|---|---|
| Markdown | **[`micromark`](https://github.com/micromark/micromark)** / **[`marked`](https://github.com/markedjs/marked)** |
| HTML parsing | **[`parse5`](https://github.com/inikulin/parse5)** / **[`cheerio`](https://github.com/cheeriojs/cheerio)** |
| YAML | **[`yaml`](https://github.com/eemeli/yaml)** |
| TOML | **[`@iarna/toml`](https://github.com/iarna/iarna-toml)** |
| Image decoding (PNG, JPEG, WebP) | Full only — native libs required. Or use **`sidecar`** to ship a CLI (ImageMagick, vips). |
| PDF parsing | **[`pdfjs-dist`](https://github.com/mozilla/pdf.js)** (large, ~1 MB). |

## Polyfill shims for Node globals

If a dependency you ship insists on Node globals:

| Missing | Shim |
|---|---|
| `Buffer` | **[`buffer`](https://github.com/feross/buffer)** (~5 KB) — API-compatible Node Buffer. |
| `process` | **[`process`](https://github.com/defunctzombie/node-process)** — minimal shim; most code just needs `process.env` which our `os.env` wraps. |
| `setImmediate` | **[`setimmediate`](https://github.com/YuzuJS/setImmediate)** — or just `setTimeout(fn, 0)`. |

## When to just switch to `full`

Some things have no acceptable pure-JS alternative:

- **HTTP/2 or HTTP/3 client** — lite's fetch bridges to `ureq` (HTTP/1.1 only); there is no pure-JS h2 client.
- **Real streaming upload backpressure** — lite drains upload bodies into memory before sending.
- **Hot-path CPU-bound JS** (image processing, big JSON parses) — the JIT gap is real.
- **Native npm bindings** (`sharp` for image resize, `better-sqlite3` for transaction throughput, `canvas` for server-side rendering).
- **Dynamic `import(path)` at runtime**.
- **`bun:ffi`, `bun:plugin`**, Chrome DevTools inspector.
- **`SharedArrayBuffer` / `Atomics`** — QuickJS limitation; use `workers` with message passing instead.

For those, flip `runtime: "full"` in `tynd.config.ts`.
