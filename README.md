<div align="center">

# **Tynd**

### Desktop apps in TypeScript. No Rust. No Go. Just TS.

[![CI](https://github.com/kvnpetit/tynd/actions/workflows/ci.yml/badge.svg)](https://github.com/kvnpetit/tynd/actions/workflows/ci.yml)
[![License](https://img.shields.io/github/license/kvnpetit/tynd?color=blue)](./LICENSE)
[![npm @tynd/cli](https://img.shields.io/npm/v/@tynd/cli?label=%40tynd%2Fcli&color=06b6d4)](https://www.npmjs.com/package/@tynd/cli)
[![npm @tynd/core](https://img.shields.io/npm/v/@tynd/core?label=%40tynd%2Fcore&color=06b6d4)](https://www.npmjs.com/package/@tynd/core)
[![npm downloads](https://img.shields.io/npm/dw/@tynd/cli?label=downloads&color=10b981)](https://www.npmjs.com/package/@tynd/cli)
[![GitHub stars](https://img.shields.io/github/stars/kvnpetit/tynd?style=flat&color=yellow)](https://github.com/kvnpetit/tynd/stargazers)

[![TypeScript](https://img.shields.io/badge/TypeScript-strict-3178c6?logo=typescript&logoColor=white)](https://www.typescriptlang.org)
[![Bun](https://img.shields.io/badge/Bun-runtime-fbf0df?logo=bun&logoColor=000)](https://bun.sh)
[![Rust](https://img.shields.io/badge/Rust-host-b7410e?logo=rust&logoColor=white)](https://www.rust-lang.org)
[![PRs welcome](https://img.shields.io/badge/PRs-welcome-brightgreen)](./CONTRIBUTING.md)
[![Last commit](https://img.shields.io/github/last-commit/kvnpetit/tynd)](https://github.com/kvnpetit/tynd/commits/main)
[![Open issues](https://img.shields.io/github/issues/kvnpetit/tynd?color=d93f0b)](https://github.com/kvnpetit/tynd/issues)

**Native window. Zero network. Full TypeScript stack.**
Same concept as Tauri and Wails — with a backend you already know.

```bash
bunx @tynd/cli create my-app
```

**→ [Getting Started in 5 minutes](./GETTING_STARTED.md)**

</div>

---

## Why Tynd

|  | Tauri v2 | Wails v3 | **Tynd** |
|---|---|---|---|
| Backend language | Rust | Go | **TypeScript** |
| Typed RPC | Rust → TS codegen | Go → TS codegen | **zero codegen** (`typeof backend`) |
| Webview | wry (native OS) | native OS | **wry (native OS)** |
| IPC | `webview_bind` | HTTP / WebSocket | **`webview_bind` + `evaluate_script`** |
| Runtimes | single | single | **full (Bun) + lite (QuickJS)** |
| Frontend | Any | Any | **Any** |

---

## How it works

```
TypeScript backend                         Native OS window
──────────────────────────                 ─────────────────────────
  export async function greet()  ◄── IPC ─ await api.greet("Alice")
  events.emit("ready", payload)  ─── push ─► api.on("ready", handler)
         │
         │ stdin/stdout JSON
         ▼
  tynd-full  (Bun subprocess, wry + tao Rust host)
  tynd-lite  (QuickJS embedded,  wry + tao Rust host)
```

**Zero network.** Frontend served via `bv://` custom protocol (wry `with_custom_protocol`). RPC via native `webview_bind`. Events via `evaluate_script`. Identical architecture to Tauri v2.

### Two runtimes

| | `lite` | `full` |
|---|---|---|
| JS engine | QuickJS (embedded in Rust binary) | Bun subprocess (JSC/JIT) |
| IPC overhead | ~0 µs (in-process) | ~0.3–1 ms (OS pipe) |
| JIT compilation | ✗ interpreter | ✓ JSC JIT |
| File system / SQLite / `fetch` | ✗ | ✓ |
| Pure-JS npm packages | ✓ (bundled) | ✓ |
| npm with native bindings | ✗ | ✓ |
| Binary size | ~5 MB smaller | Larger (runtime embedded) |
| Startup | ~20 ms | ~80 ms |

→ See [`RUNTIMES.md`](RUNTIMES.md) for the full comparison (APIs, performance, detection, examples).

---

## Requirements

**[Bun](https://bun.sh) is required.** Tynd is a Bun-first framework — the CLI, the dev server, and the full runtime all run on Bun.

End users who install your app need nothing — the runtime is embedded in the distributed binary.

---

## Quick start

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

## CLI

```bash
tynd create [name]           # scaffold a new project (interactive if no args)
  --framework react|vue|svelte|solid|preact|lit|angular
  --runtime   full|lite

tynd dev                     # start app in development mode (HMR)
tynd start                   # classic JS build (frontend + backend) then run (no HMR)
tynd build                   # bundle backend + frontend → single binary
  --bundle [targets]         # + installers: app, dmg, deb, rpm, appimage, nsis, msi (or "all")
tynd init                    # add tynd to an existing project
tynd clean                   # remove build artifacts (.tynd/cache, release/)
tynd validate                # check config and project structure
tynd upgrade                 # upgrade @tynd/cli and @tynd/core to latest
tynd info                    # show environment info (Bun, Rust, WebView2…)
```

---

## Supported frameworks

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

## Project structure

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

## Backend API

### `app.start(config)`

Called once at the bottom of your backend entry file.

```typescript
import { app, createEmitter } from "@tynd/core"

// 1. Declare typed events
export const events = createEmitter<{
  userCreated: { id: string; name: string }
}>()

// 2. Export functions — each becomes callable from the frontend
export async function greet(name: string): Promise<string> {
  return `Hello, ${name}!`
}

// 3. Lifecycle hooks
app.onReady(() => {
  events.emit("userCreated", { id: "1", name: "Alice" })
})

app.onClose(() => {
  // cleanup
})

// 4. Start
app.start({
  frontendDir: import.meta.dir + "/../dist",
  window: {
    title:   "My App",
    width:   1200,
    height:  800,
    center:  true,
  },
})
```

### `AppConfig`

| Field | Type | Description |
|---|---|---|
| `window` | `WindowConfig` | Window options |
| `frontendDir` | `string` | Path to built frontend assets |
| `devUrl` | `string` | Dev server URL (auto-detected; overrides `frontendDir` in dev) |
| `menu` | `MenuSubmenu[]` | Native menu bar |
| `tray` | `TrayConfig` | System tray |

### `WindowConfig`

| Field | Default | Description |
|---|---|---|
| `title` | `""` | Window title |
| `width` | `1200` | Initial width |
| `height` | `800` | Initial height |
| `minWidth` / `minHeight` | — | Minimum size |
| `maxWidth` / `maxHeight` | — | Maximum size |
| `resizable` | `true` | Allow resize |
| `decorations` | `true` | Show title bar |
| `transparent` | `false` | Transparent background |
| `alwaysOnTop` | `false` | Pin above other windows |
| `center` | `false` | Center on screen at startup |
| `fullscreen` | `false` | Start fullscreen |
| `maximized` | `false` | Start maximized |

### `createEmitter<T>()`

Create a typed event bus. Export the result — the frontend subscribes via `api.on()`.

```typescript
export const events = createEmitter<{
  fileChanged: { path: string }
  progress:    { percent: number }
}>()

events.emit("fileChanged", { path: "/foo.ts" })
```

### Native menu bar

```typescript
app.start({
  menu: [
    {
      type: "submenu",
      label: "File",
      items: [
        { label: "New",  id: "file.new" },
        { label: "Open", id: "file.open" },
        { type: "separator" },
        { role: "quit" },
      ],
    },
    {
      type: "submenu",
      label: "Edit",
      items: [
        { role: "undo" }, { role: "redo" },
        { type: "separator" },
        { role: "cut" }, { role: "copy" }, { role: "paste" },
      ],
    },
  ],
  // ...
})
```

### System tray

```typescript
app.start({
  tray: {
    icon:    import.meta.dir + "/assets/tray.png",
    tooltip: "My App",
    menu: [
      { label: "Show",  id: "show" },
      { label: "Quit",  id: "quit" },
    ],
  },
  // ...
})
```

---

## Frontend API

```typescript
import { createBackend } from "@tynd/core/client"
import type * as backend from "../../backend/main"

const api = createBackend<typeof backend>()

// Call backend functions (fully typed)
const msg = await api.greet("Alice")   // string ✅

// Subscribe to backend events
api.on("userCreated", (user) => console.log(user.name))

// Once (auto-unsubscribes)
api.once("userCreated", (user) => { /* fires once */ })
```

### OS APIs

All OS APIs are called from the frontend. They go through the Rust host directly — no round-trip to the TypeScript backend.

#### `dialog`

```typescript
import { dialog } from "@tynd/core/client"

const path  = await dialog.openFile({ filters: [{ name: "Images", extensions: ["png", "jpg"] }] })
const paths = await dialog.openFiles()
const dest  = await dialog.saveFile({ defaultPath: "export.csv" })
await dialog.message("Done!", { title: "Success" })
const ok    = await dialog.confirm("Delete this file?")
```

#### `tyndWindow`

```typescript
import { tyndWindow } from "@tynd/core/client"

await tyndWindow.setTitle("My App — Unsaved")
await tyndWindow.setSize(1400, 900)
await tyndWindow.maximize()
await tyndWindow.unmaximize()
await tyndWindow.minimize()
await tyndWindow.unminimize()
await tyndWindow.center()
await tyndWindow.setFullscreen(true)
await tyndWindow.setAlwaysOnTop(true)
await tyndWindow.setDecorations(false)
await tyndWindow.hide()
await tyndWindow.show()

const isMax  = await tyndWindow.isMaximized()   // boolean
const isMin  = await tyndWindow.isMinimized()   // boolean
const isFull = await tyndWindow.isFullscreen()  // boolean
const isVis  = await tyndWindow.isVisible()     // boolean

// Subscribe to native menu item by id
tyndWindow.onMenu("file.new", () => newDocument())
```

#### `clipboard`

```typescript
import { clipboard } from "@tynd/core/client"

const text = await clipboard.readText()
await clipboard.writeText("Hello!")
```

#### `shell`

```typescript
import { shell } from "@tynd/core/client"

await shell.openExternal("https://example.com")
await shell.openPath("/home/user/document.pdf")
```

#### `notification`

```typescript
import { notification } from "@tynd/core/client"

await notification.send("Build Complete", { body: "0 errors." })
```

#### `tray`

```typescript
import { tray } from "@tynd/core/client"

tray.onClick(() => tyndWindow.show())
tray.onRightClick(() => { /* ... */ })
tray.onDoubleClick(() => tyndWindow.show())
tray.onMenu("quit", () => process.exit(0))
```

---

## IPC architecture

| Channel | Direction | Transport |
|---|---|---|
| `api.<fn>()` | Frontend → Backend | wry `window.ipc.postMessage` → stdin JSON |
| `api.on()` | Backend → Frontend | Rust `evaluate_script` |
| `events.emit()` | Backend → Frontend | stdout JSON → Rust → `evaluate_script` |
| `dialog` / `tyndWindow` / … | Frontend → Rust | wry `window.ipc.postMessage` (direct, no backend round-trip) |

**No HTTP. No WebSocket. No firewall prompt.** Identical to Tauri v2's IPC stack.

Frontend assets served via `bv://localhost/` (wry custom protocol → filesystem). `window.location.origin` is `bv://localhost`.

---

## tynd.config.ts

```typescript
import type { TyndConfig } from "@tynd/cli"

export default {
  runtime:     "full",              // "full" | "lite"
  backend:     "backend/main.ts",   // backend entry file
  frontendDir: "dist",              // built frontend assets (relative to project root)
  icon:        "public/favicon.png", // optional — auto-detected if omitted
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

**Icon auto-detection order:** `public/favicon.ico` → `public/favicon.png` → `public/icon.ico` → `public/icon.png` → `public/logo.ico` → `public/logo.png` → SVG variants (`public/favicon.svg`, `public/icon.svg`, `public/logo.svg`) → `assets/icon.{ico,png}` → `icon.{ico,png}`. SVG icons are auto-converted to PNG via WASM. Set `icon` explicitly to override.

---

## WebView runtime

| OS | WebView | Pre-installed? |
|---|---|---|
| Windows 10/11 | WebView2 (Edge Chromium) | ✅ |
| macOS | WKWebView | ✅ |
| Linux | WebKitGTK 4.1 | ⚠️ `sudo apt install libwebkit2gtk-4.1-0` |

---

## Building from source

The `tynd-full` and `tynd-lite` binaries are Rust crates.

```bash
# Requirements
rustup install stable

# Linux only
sudo apt install libgtk-3-dev libwebkit2gtk-4.1-dev \
  libjavascriptcoregtk-4.1-dev libsoup-3.0-dev

# Build
cargo build --release -p tynd-full
cargo build --release -p tynd-lite
```

### Crate layout

```
packages/
├── host-rs/     ← tynd-host (library: wry + tao event loop, OS APIs)
├── full/        ← tynd-full (binary: spawns Bun subprocess, links host)
├── lite/        ← tynd-lite (binary: embeds QuickJS, links host)
├── host/        ← @tynd/host (npm: postinstall downloads pre-built binaries)
├── core/        ← @tynd/core (TypeScript: app, createEmitter, client API)
└── cli/         ← @tynd/cli  (TypeScript: tynd create/dev/build/info)
```
