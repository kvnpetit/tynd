<div align="center">

# **Tynd**

### Desktop apps in TypeScript. One language, native binary, no glue code.

[![CI](https://github.com/kvnpetit/tynd/actions/workflows/ci.yml/badge.svg)](https://github.com/kvnpetit/tynd/actions/workflows/ci.yml)
[![License](https://img.shields.io/github/license/kvnpetit/tynd?color=blue)](./LICENSE)
[![npm @tynd/cli](https://img.shields.io/npm/v/@tynd/cli?label=%40tynd%2Fcli&color=06b6d4)](https://www.npmjs.com/package/@tynd/cli)
[![npm @tynd/core](https://img.shields.io/npm/v/@tynd/core?label=%40tynd%2Fcore&color=06b6d4)](https://www.npmjs.com/package/@tynd/core)
[![npm downloads](https://img.shields.io/npm/dw/@tynd/cli?label=downloads&color=10b981)](https://www.npmjs.com/package/@tynd/cli)
[![GitHub stars](https://img.shields.io/github/stars/kvnpetit/tynd?style=flat&color=yellow)](https://github.com/kvnpetit/tynd/stargazers)

[![TypeScript](https://img.shields.io/badge/TypeScript-strict-3178c6?logo=typescript&logoColor=white)](https://www.typescriptlang.org)
[![Bun](https://img.shields.io/badge/Bun-runtime-fbf0df?logo=bun&logoColor=000)](https://bun.sh)
[![PRs welcome](https://img.shields.io/badge/PRs-welcome-brightgreen)](./CONTRIBUTING.md)
[![Last commit](https://img.shields.io/github/last-commit/kvnpetit/tynd)](https://github.com/kvnpetit/tynd/commits/main)
[![Open issues](https://img.shields.io/github/issues/kvnpetit/tynd?color=d93f0b)](https://github.com/kvnpetit/tynd/issues)

**Native window. Zero network. TypeScript end-to-end.**
Write your backend and frontend in TypeScript, ship a small native binary — no extra language to learn, no codegen step.

```bash
bunx @tynd/cli create my-app
```

**-> [Getting Started in 5 minutes](./GETTING_STARTED.md)**

</div>

---

## ✨ At a glance

- 🟦 **TypeScript top to bottom.** Backend, frontend, IPC, config — same language, no codegen.
- 🧬 **Two runtime modes, one API.** Start with `lite` (~6.5 MB native binary, no extra runtime needed). Switch to `full` with one config line when you need Bun's JIT or native-binding npm packages.
- 🔒 **Native window, zero network.** No HTTP server, no loopback TCP port, no firewall prompt. Frontend and IPC ride a native custom scheme.
- 🧰 **30 OS APIs, identical in both modes** — `fs` (+ watcher, handles, symlinks, trash), `sql` (embedded SQLite), `http`, `websocket`, `terminal` (real PTY), `compute` (blake3/sha/CSPRNG), `dialog`, `tray` (runtime mutation + mouse enter/leave), `menu` (accelerators + checkbox/radio + context menu), `notification` (icon/sound/actions/schedule), `clipboard` (image + HTML + change monitor), `shell` (incl. reveal), `process`, `sidecar`, `singleInstance` (with argv forwarding + deep links), `shortcuts` (system-wide hotkeys), `keyring` (OS-encrypted secrets), `autolaunch` (start at boot), `store`, `updater` (signed auto-update + periodic check), `workers`, `log` (JSON-lines + rotation + console capture), `power` (idle time), `security` (capability ACL for fs + http), `app` (name/version/exit/relaunch), `os` / `path`, `tyndWindow` (multi-window + events + cursor + backdrop), plus typed emitters (bidirectional) and streaming RPC.
- ⚡ **Zero-copy binary IPC.** Multi-MB payloads (`fs.readBinary`, `compute.hash`, …) skip JSON/base64 entirely — roughly **5-10× faster** than the usual webview-framework binary path.
- 📦 **First-class installers.** `tynd build --bundle` emits `.app` / `.dmg` / `.deb` / `.rpm` / `.AppImage` / NSIS `.exe` / `.msi`. Installer tools auto-download on first build — no manual setup. Built-in code signing (`signtool` on Windows, `codesign` + optional notarization on macOS).
- 🛡️ **Security defaults.** Auto-injected CSP on every HTML response, OS-backed secret storage via `keyring`, Ed25519-signed auto-updater so tampered binaries are rejected.
- 🌊 **Streaming RPC that doesn't flinch.** Async-generator backend handlers stream to the frontend with per-chunk flow control + batched DOM updates — 10k+ yields/sec without freezing the UI.
- 🎨 **Framework-agnostic.** React, Vue, Svelte, Solid, Preact, Lit, Angular — anything that outputs a pure SPA.

---

## 🧭 Why Tynd

- **TypeScript backend, no codegen.** The frontend types backend calls from `typeof backend` — no `.d.ts` generation, no IDL, no schema file.
- **Native OS webview.** The final binary doesn't ship a browser — it uses WebView2 / WKWebView / WebKitGTK, so you inherit the OS's paint loop, font stack, and accessibility for free.
- **Two modes, one API.** `lite` for the smallest binary (~6.5 MB, embedded JS engine). `full` when you need Bun's JIT or native-binding npm packages. Switch via one config line — every OS API works the same in both.
- **Zero network IPC.** RPC and assets never touch TCP — no loopback port, no firewall prompt on first launch.

See [COMPARISON.md](./COMPARISON.md) for the full Tynd vs Tauri / Wails / Electron matrix (39 categories, 500+ rows).

---

## 🧪 How it works

```
TypeScript backend                         Native OS window
──────────────────────────                 ─────────────────────────
  export async function greet()  ◄── IPC ─ await api.greet("Alice")
  events.emit("ready", payload)  ─── push ─► api.on("ready", handler)
         │
         ▼
  tynd-full  — your TypeScript runs on Bun, wrapped in a native host
  tynd-lite  — your TypeScript runs inside the native host, no extra runtime
```

**Zero network.** Frontend assets and IPC ride a native custom scheme — no HTTP server, no loopback port, no firewall prompt. Multi-MB binary payloads (`fs.readBinary`, `fs.writeBinary`, `compute.hash`, …) skip JSON entirely via a dedicated binary channel.

### Two runtime modes

| | `lite` | `full` |
|---|---|---|
| JS runtime | embedded interpreter — ships inside the native binary | Bun, packed into the native binary |
| Hot JS speed | interpreter — fine for IPC glue, slower on tight loops | **Bun JIT — often 10-100× faster on CPU-bound JS** |
| IPC overhead | in-process, no serialization hop | one serialization hop (OS pipe) |
| `fs` / `http` / `websocket` / `sql` / `compute` / `terminal` / … | ✓ same API | ✓ same API |
| JS-level `fetch` / `Bun.file` / `bun:sqlite` | ✗ (use the Tynd API) | ✓ |
| Pure-JS npm packages | ✓ (bundled) | ✓ |
| npm with native bindings | ✗ | ✓ |
| Binary size | smaller (~6.5 MB) | larger (~44 MB, Bun compressed) |
| Startup | faster (everything in-process) | slower (spawns Bun) |

-> See [`RUNTIMES.md`](RUNTIMES.md) for the full comparison (APIs, performance, detection, examples).

---

## 📋 Requirements

**[Bun](https://bun.sh) is required for app developers.** Tynd is a Bun-first framework — the CLI, the dev server, and the full runtime all run on Bun. Node.js is not supported as a replacement.

```bash
# macOS / Linux / WSL
curl -fsSL https://bun.sh/install | bash

# Windows (PowerShell)
powershell -c "irm bun.sh/install.ps1 | iex"
```

**End users of your built app need nothing** — whichever runtime mode you picked is already packed into the distributed binary.

---

## 🚀 Quick start

```bash
bunx tynd create my-app
cd my-app
bun run dev
```

With a specific framework / runtime:

```bash
bunx tynd create my-app --framework react --runtime full
bunx tynd create my-app --framework vue   --runtime lite
```

---

## 🛠️ CLI

```bash
tynd create [name]           # scaffold a new project (interactive if no args)
  --framework react|vue|svelte|solid|preact|lit|angular
  --runtime   full|lite

tynd dev                     # start app in development mode (HMR)
tynd start                   # classic JS build (frontend + backend) then run (no HMR)
tynd build                   # bundle backend + frontend -> single binary
  --bundle [targets]         # + installers: app, dmg, deb, rpm, appimage, nsis, msi (or "all")
tynd init                    # add tynd to an existing project
tynd clean                   # remove build artifacts (.tynd/cache, release/)
tynd validate                # check config and project structure
tynd upgrade                 # upgrade @tynd/cli and @tynd/core to latest
tynd info                    # show environment info (Bun version, WebView, paths)
tynd keygen                  # generate an Ed25519 keypair for the auto-updater
tynd sign <file>             # sign a file with an updater private key
```

---

## 🎨 Supported frameworks

| Framework | Scaffold | Build | Fast Refresh (HMR) |
|---|---|---|---|
| React                 | ✅ Vite `react-ts`  | ✅ | ⚠ OK; breaks if React Compiler is enabled |
| Vue / Svelte / Solid / Preact | ✅ Vite `<name>-ts` | ✅ | ✅ |
| Lit                   | ✅ Vite `lit-ts`    | ✅ | ♻ Full reload only — Web Components by design |
| Angular               | ✅ Angular CLI      | ✅ | ♻ Full reload by default (opt-in HMR via `ng serve --hmr`) |

`tynd init` also detects existing **Vite**, **CRA**, **Angular CLI**, **Parcel**, **Rsbuild**, and **Webpack** setups.

**Blocked (SSR):** Next.js, Nuxt, SvelteKit, Remix, Gatsby, Blitz.js, RedwoodJS, SolidStart, Angular Universal, Analog, Qwik City, Astro, TanStack Start, Vike. Use the SPA variant instead (plain Svelte vs SvelteKit, plain Solid vs SolidStart, …).

See [`FRAMEWORKS.md`](FRAMEWORKS.md) for the full matrix, per-framework notes, output-dir rules, and the React Compiler workaround.

---

## 📁 Project structure

```
my-app/
├── tynd.config.ts            ← project config
├── package.json
├── backend/
│   └── main.ts               ← backend entry — app.start() here
└── src/                      ← frontend source (React / Vue / Svelte…)
    └── main.tsx
```

---

## 🧩 API

Tynd exposes three surfaces — backend module, typed frontend RPC, and direct OS APIs:

| Surface | Import | Purpose |
|---|---|---|
| **Backend** | `@tynd/core` | `app.start`, `app.onReady`, `app.onClose`, `createEmitter` |
| **Frontend RPC** | `@tynd/core/client` | `createBackend<typeof backend>()` — typed proxy |
| **OS APIs** | `@tynd/core/client` | `app`, `dialog`, `tyndWindow`, `monitors`, `menu`, `clipboard`, `shell`, `notification`, `tray`, `process`, `fs`, `shortcuts`, `keyring`, `autolaunch`, `store`, `updater`, `log`, `power`, `security`, `os`, `path`, `http`, `websocket`, `sql`, `sidecar`, `terminal`, `compute`, `workers`, `singleInstance` |
| **Web APIs** | `@tynd/core/client` | `fetch`, `WebSocket`, `EventSource`, `crypto`, `URL`, `Blob`, `FormData`, `AbortController`, `TextEncoder`, … (re-exports for `import * as tynd`) |

Short example:

```typescript
// backend/main.ts
import { app, createEmitter } from "@tynd/core"

export const events = createEmitter<{ ready: { message: string } }>()
export async function greet(name: string): Promise<string> { return `Hello, ${name}!` }

app.start({
  frontendDir: import.meta.dir + "/../dist",
  window: { title: "My App", width: 1200, height: 800, center: true },
})
```

```typescript
// src/App.tsx
import { createBackend, fs, process, terminal } from "@tynd/core/client"
import type * as backend from "../backend/main"

const api = createBackend<typeof backend>()
const msg = await api.greet("Alice")   // string ✅

await fs.writeText("data.json", JSON.stringify(state))
const { stdout } = await process.exec("git", { args: ["status"] })
```

**Full reference:** [API.md](./API.md) — every method, signature, and example.

---

## ⚙️ tynd.config.ts

```typescript
import type { TyndConfig } from "@tynd/cli"

export default {
  runtime:     "full",              // "full" | "lite"
  backend:     "backend/main.ts",   // backend entry file
  frontendDir: "dist",              // built frontend assets (relative to project root)
  icon:        "public/favicon.svg", // optional — auto-detected; SVG recommended
  window: {
    title:  "My App",
    width:  1200,
    height: 800,
    center: true,
  },
} satisfies TyndConfig
```

### `TyndConfig` fields

| Field | Default | Description |
|---|---|---|
| `runtime` | `"full"` | `"full"` or `"lite"` — see [RUNTIMES.md](RUNTIMES.md) |
| `backend` | `"backend/main.ts"` | Backend entry file |
| `frontendDir` | `"frontend"` | Built frontend output directory |
| `frontendEntry` | — | Simple TS/JS entry (no framework) — auto-bundled by tynd |
| `devUrl` | auto | Dev server URL override |
| `devCommand` | auto | Dev server start command override |
| `icon` | auto | App icon path — auto-detected from `public/favicon.{ico,png,svg}` |
| `binaryArgs` | — | Extra args passed to the `tynd-full` / `tynd-lite` binary |
| `window` | — | Default window options (title, width, height, center) |

**Icon auto-detection order** (SVG first — single source of truth renders pixel-perfect at every size):
`public/{favicon,icon,logo}.svg` -> `public/{favicon,icon,logo}.{ico,png}` -> `assets/icon.{svg,png,ico}` -> `icon.{svg,png,ico}`. One source feeds every output — each build produces a multi-size ICO (16/32/48/256) for Windows, a multi-entry ICNS (32/128/256/512/1024) for macOS `.app`, and a full hicolor PNG tree (16..512) for `.deb` / `.rpm` / `.AppImage`. PNG sources degrade to single-size; `.ico` sources pass through to Windows bundles and are skipped (with a warning) for macOS/Linux. Set `icon` explicitly to override.

---

## 🪟 WebView runtime

| OS | WebView | Pre-installed? |
|---|---|---|
| Windows 10/11 | WebView2 (Edge Chromium) | ✅ |
| macOS | WKWebView | ✅ |
| Linux | WebKitGTK 4.1 | ⚠️ `sudo apt install libwebkit2gtk-4.1-0` |

---

## 🏗️ Building from source (contributors only)

App authors don't need this section — `@tynd/host`'s postinstall downloads pre-built binaries automatically. If you want to build the native host yourself:

```bash
# Requirements
rustup install stable

# Linux only
sudo apt install libgtk-3-dev libwebkit2gtk-4.1-dev \
  libjavascriptcoregtk-4.1-dev libsoup-3.0-dev \
  libxdo-dev

# Build
cargo build --release -p tynd-full
cargo build --release -p tynd-lite
```

### Repo layout

```
packages/
├── host-rs/     ← native host library (window + IPC + OS APIs)
├── full/        ← tynd-full binary (packs Bun inside the host)
├── lite/        ← tynd-lite binary (embeds a lightweight JS engine)
├── host/        ← @tynd/host (npm: postinstall downloads pre-built binaries)
├── core/        ← @tynd/core (TypeScript: app, createEmitter, client API)
└── cli/         ← @tynd/cli  (TypeScript: tynd create/dev/build/info)
```

---

## 📚 Documentation

**-> [tynd.kvnpetit.com/docs](https://tynd.kvnpetit.com/docs)** — live versioned docs site.

| Doc | When to read it |
|---|---|
| [GETTING_STARTED.md](./GETTING_STARTED.md) | First project — scaffold, dev, build, ship in 5 min |
| [API.md](./API.md) | Every backend / frontend / OS API with signatures + examples |
| [RUNTIMES.md](./RUNTIMES.md) | `lite` vs `full` — what each exposes, when to pick which |
| [FRAMEWORKS.md](./FRAMEWORKS.md) | Per-framework matrix (React, Vue, Svelte, Solid, Angular, Preact, Lit) |
| [COMPARISON.md](./COMPARISON.md) | Tynd vs Tauri / Wails / Electron across 39 categories |
| [SIGNING.md](./SIGNING.md) | Code signing + notarization workflows per OS |
| [TROUBLESHOOTING.md](./TROUBLESHOOTING.md) | Common errors: missing binary, WebView2, GTK deps, Gatekeeper, etc. |
| [CONTRIBUTING.md](./CONTRIBUTING.md) | Dev loop, commit style, PR process |
| Per-package READMEs | [@tynd/core](./packages/core/README.md) · [@tynd/cli](./packages/cli/README.md) · [@tynd/host](./packages/host/README.md) |
| Working examples | [playground/full](./playground/full/README.md) (LLM chatbot, full runtime) · [playground/example](./playground/example/README.md) (minimal demo, lite runtime) |
| [docs/](./docs/README.md) | Next.js 16 + Nextra 4 docs site (landing + versioned docs). Static export, deploys to Cloudflare Pages. `bun --cwd docs run dev` |
