# Vorn

Native desktop apps with **TypeScript** — same concept as Tauri/Wails but the backend is pure TS.

| | Tauri v2 | Wails v3 | **Vorn** |
|---|---|---|---|
| Backend language | Rust | Go | **TypeScript (full or lite runtime)** |
| Frontend | Any | Any | Any |
| Typed RPC | Rust → TS codegen | Go → TS codegen | **zero codegen** — `typeof backend` |
| Webview | wry (native OS) | native OS | **wry (native OS)** |
| IPC | `webview_bind` | HTTP/WS | **`webview_bind` + `evaluate_script`** |

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
  vorn-full  (Bun subprocess, wry + tao Rust host)
  vorn-lite  (QuickJS embedded,  wry + tao Rust host)
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

→ See [`docs/runtimes.md`](docs/runtimes.md) for the full comparison (APIs, performance, detection, examples).

---

## Requirements

**[Bun](https://bun.sh) is required.** Vorn is a Bun-first framework — the CLI, the dev server, and the full runtime all run on Bun.

End users who install your app need nothing — the runtime is embedded in the distributed binary.

---

## Quick start

```bash
bunx vorn create my-app
cd my-app
bun run dev
```

With a specific framework / runtime:

```bash
bunx vorn create my-app --framework react --runtime full
bunx vorn create my-app --framework vue   --runtime lite
```

---

## CLI

```bash
vorn create [name]           # scaffold a new project (interactive if no args)
  --framework react|vue|svelte|solid|preact|lit|angular
  --runtime   full|lite

vorn dev                     # start app in development mode (HMR)
vorn build                   # bundle backend + frontend → single binary
vorn init                    # add vorn to an existing project
vorn clean                   # remove build artifacts (.vorn/cache, release/)
vorn validate                # check config and project structure
vorn upgrade                 # upgrade @vorn/cli and @vorn/core to latest
vorn info                    # show environment info (Bun, Rust, WebView2…)
```

---

## Project structure

```
my-app/
├── vorn.config.ts            ← project config
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
import { app, createEmitter } from "@vorn/core"

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
import { createBackend } from "@vorn/core/client"
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
import { dialog } from "@vorn/core/client"

const path  = await dialog.openFile({ filters: [{ name: "Images", extensions: ["png", "jpg"] }] })
const paths = await dialog.openFiles()
const dest  = await dialog.saveFile({ defaultPath: "export.csv" })
await dialog.message("Done!", { title: "Success" })
const ok    = await dialog.confirm("Delete this file?")
```

#### `vornWindow`

```typescript
import { vornWindow } from "@vorn/core/client"

await vornWindow.setTitle("My App — Unsaved")
await vornWindow.setSize(1400, 900)
await vornWindow.maximize()
await vornWindow.unmaximize()
await vornWindow.minimize()
await vornWindow.unminimize()
await vornWindow.center()
await vornWindow.setFullscreen(true)
await vornWindow.setAlwaysOnTop(true)
await vornWindow.setDecorations(false)
await vornWindow.hide()
await vornWindow.show()

const isMax  = await vornWindow.isMaximized()   // boolean
const isMin  = await vornWindow.isMinimized()   // boolean
const isFull = await vornWindow.isFullscreen()  // boolean
const isVis  = await vornWindow.isVisible()     // boolean

// Subscribe to native menu item by id
vornWindow.onMenu("file.new", () => newDocument())
```

#### `clipboard`

```typescript
import { clipboard } from "@vorn/core/client"

const text = await clipboard.readText()
await clipboard.writeText("Hello!")
```

#### `shell`

```typescript
import { shell } from "@vorn/core/client"

await shell.openExternal("https://example.com")
await shell.openPath("/home/user/document.pdf")
```

#### `notification`

```typescript
import { notification } from "@vorn/core/client"

await notification.send("Build Complete", { body: "0 errors." })
```

#### `tray`

```typescript
import { tray } from "@vorn/core/client"

tray.onClick(() => vornWindow.show())
tray.onRightClick(() => { /* ... */ })
tray.onDoubleClick(() => vornWindow.show())
tray.onMenu("quit", () => process.exit(0))
```

---

## IPC architecture

| Channel | Direction | Transport |
|---|---|---|
| `api.<fn>()` | Frontend → Backend | wry `window.ipc.postMessage` → stdin JSON |
| `api.on()` | Backend → Frontend | Rust `evaluate_script` |
| `events.emit()` | Backend → Frontend | stdout JSON → Rust → `evaluate_script` |
| `dialog` / `vornWindow` / … | Frontend → Rust | wry `window.ipc.postMessage` (direct, no backend round-trip) |

**No HTTP. No WebSocket. No firewall prompt.** Identical to Tauri v2's IPC stack.

Frontend assets served via `bv://localhost/` (wry custom protocol → filesystem). `window.location.origin` is `bv://localhost`.

---

## vorn.config.ts

```typescript
import type { VornConfig } from "@vorn/cli"

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
} satisfies VornConfig
```

### `VornConfig` fields

| Field | Default | Description |
|---|---|---|
| `runtime` | `"full"` | `"full"` or `"lite"` — see [docs/runtimes.md](docs/runtimes.md) |
| `backend` | `"backend/main.ts"` | Backend entry file |
| `frontendDir` | `"frontend"` | Built frontend output directory |
| `frontendEntry` | — | Simple TS/JS entry (no framework) — auto-bundled by vorn |
| `devUrl` | auto | Dev server URL override |
| `devCommand` | auto | Dev server start command override |
| `icon` | auto | App icon path — auto-detected from `public/favicon.{ico,png,svg}` |
| `binaryArgs` | — | Extra args passed to the `vorn-full` / `vorn-lite` binary |
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

The `vorn-full` and `vorn-lite` binaries are Rust crates.

```bash
# Requirements
rustup install stable

# Linux only
sudo apt install libgtk-3-dev libwebkit2gtk-4.1-dev

# Build
cargo build --release -p vorn-full
cargo build --release -p vorn-lite
```

### Crate layout

```
packages/
├── host/        ← vorn-host (library: wry + tao event loop, OS APIs)
├── full/        ← vorn-full (binary: spawns Bun subprocess, links host)
├── lite/        ← vorn-lite (binary: embeds QuickJS, links host)
├── core/        ← @vorn/core (TypeScript: app, createEmitter, client API)
└── cli/         ← @vorn/cli  (TypeScript: vorn create/dev/build/info)
```
