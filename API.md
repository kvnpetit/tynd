# 📚 Tynd API Reference

Every Tynd app has three surfaces:

- **Backend** (`@tynd/core`) — imported by `backend/main.ts`, exposes `app.start`, emitters, lifecycle hooks.
- **Frontend RPC** (`@tynd/core/client`) — typed proxy to your backend functions (`createBackend<typeof backend>()`).
- **OS APIs** (`@tynd/core/client`) — direct bridge from the frontend to Rust: dialog, window, clipboard, shell, notification, tray, process, fs, http, sidecar, terminal, store, os, path.

> **Lite vs full parity:** all OS APIs live in Rust (`packages/host-rs/src/os/`) so both runtimes expose the exact same surface. See [`RUNTIMES.md`](./RUNTIMES.md) for JS-runtime differences.

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
})
```

---

## Frontend RPC — `createBackend<T>()`

```typescript
import { createBackend } from "@tynd/core/client"
import type * as backend from "../../backend/main"

const api = createBackend<typeof backend>()

const msg = await api.greet("Alice")   // string ✅ — fully typed
api.on("userCreated", (user) => console.log(user.name))
api.once("userCreated", (user) => { /* fires once */ })
```

Types come from `typeof backend` — rename a backend function and the compiler catches the frontend call.

---

## OS APIs

Called from the frontend. They go through the Rust host directly — no round-trip to the TypeScript backend.

### `dialog`

```typescript
import { dialog } from "@tynd/core/client"

const path  = await dialog.openFile({ filters: [{ name: "Images", extensions: ["png", "jpg"] }] })
const paths = await dialog.openFiles()
const dest  = await dialog.saveFile({ defaultPath: "export.csv" })
await dialog.message("Done!", { title: "Success" })
const ok    = await dialog.confirm("Delete this file?")
```

### `tyndWindow`

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

const isMax  = await tyndWindow.isMaximized()
const isMin  = await tyndWindow.isMinimized()
const isFull = await tyndWindow.isFullscreen()
const isVis  = await tyndWindow.isVisible()

tyndWindow.onMenu("file.new", () => newDocument())
```

### `clipboard`

```typescript
import { clipboard } from "@tynd/core/client"

const text = await clipboard.readText()
await clipboard.writeText("Hello!")
```

### `shell`

```typescript
import { shell } from "@tynd/core/client"

await shell.openExternal("https://example.com")
await shell.openPath("/home/user/document.pdf")
```

### `notification`

```typescript
import { notification } from "@tynd/core/client"

await notification.send("Build Complete", { body: "0 errors." })
```

### `tray`

```typescript
import { tray } from "@tynd/core/client"

tray.onClick(() => tyndWindow.show())
tray.onRightClick(() => { /* ... */ })
tray.onDoubleClick(() => tyndWindow.show())
tray.onMenu("quit", () => process.exit(0))
```

### `process`

```typescript
import { process } from "@tynd/core/client"

const { stdout, code } = await process.exec("git", { args: ["status", "--short"] })
const full = await process.execShell("ls -la | grep tynd")
```

### `fs`

```typescript
import { fs } from "@tynd/core/client"

await fs.writeText("data.json", JSON.stringify(state), { createDirs: true })
const text = await fs.readText("data.json")
const entries = await fs.readDir(".")
const info = await fs.stat("data.json")

const bytes = await fs.readBinary("image.png")
await fs.writeBinary("copy.png", bytes)
```

### `store`

```typescript
import { createStore } from "@tynd/core/client"

const prefs = createStore("com.example.myapp")
await prefs.set("theme", "dark")
const theme = await prefs.get<string>("theme")
```

JSON-backed k/v under the OS config dir.

### `os` / `path`

```typescript
import { os, path } from "@tynd/core/client"

const { platform, arch } = await os.info()
const home = await os.homeDir()
const cfgFile = path.join(await os.configDir() ?? "", "myapp", "config.json")
```

### `http`

```typescript
import { http } from "@tynd/core/client"

const { body, status } = await http.getJson<Repo[]>("https://api.github.com/users/kvnpetit/repos")
const { body: html } = await http.get("https://example.com")
const { body: bytes } = await http.getBinary("https://example.com/image.png")

await http.post("https://api.example.com/events", {
  body: { name: "click" },
  headers: { authorization: "Bearer …" },
})

await http.download("https://…/ffmpeg.zip", "./downloads/ffmpeg.zip", {
  onProgress: ({ loaded, total }) => {
    console.log(total ? `${((loaded / total) * 100).toFixed(1)}%` : `${loaded}B`)
  },
})
```

`onProgress` fires for both upload (when `body` is set) and download (when reading the response body). Throttled to ~50ms max. Pure-Rust TLS (rustls) — no OpenSSL runtime dep.

### `sidecar` — bundled binaries

Declare side-loaded binaries in `tynd.config.ts`:

```ts
sidecars: [
  { name: "ffmpeg.exe", path: "bin/ffmpeg.exe" },
]
```

Embedded into the built `.exe` and extracted to a per-launch temp dir. At runtime:

```typescript
import { sidecar, process } from "@tynd/core/client"

const ffmpeg = await sidecar.path("ffmpeg.exe")
const { stdout } = await process.exec(ffmpeg, { args: ["-version"] })
```

### `terminal` — real PTY inside the app

```typescript
import { terminal } from "@tynd/core/client"

const t = await terminal.spawn({ cols: 120, rows: 30 })  // uses $SHELL / COMSPEC

t.onData((bytes) => xterm.write(bytes))                  // wire into xterm.js
t.onExit((code) => console.log("shell exited:", code))

await t.write("ls -la\n")
await t.resize(140, 40)
await t.kill()
```

Backed by `portable-pty` (ConPTY on Windows, POSIX PTY elsewhere). Pair with [xterm.js](https://xtermjs.org/) for a full interactive terminal.

### `compute` — Rust-native CPU helpers

```typescript
import { compute } from "@tynd/core/client"

const digest = await compute.hash("hello world", { algo: "blake3" })    // hex string
const sha = await compute.hash(bytes, { algo: "sha256", encoding: "base64" })

const squeezed = await compute.compress(payload, { algo: "zstd", level: 9 })
const restored = await compute.decompress(squeezed)
```

Runs on a fresh Rust thread per call — never blocks the JS event loop. Works identically in lite and full.

### `workers` — offload CPU-bound JS

```typescript
import { workers, parallel } from "@tynd/core/client"

const w = await workers.spawn((input: number[]) => input.reduce((a, b) => a + b, 0))
const sum = await w.run<number, number[]>([1, 2, 3, 4, 5])
await w.terminate()

const results = await parallel.map(
  bigArray,
  (item) => heavyCpuWork(item),
  { concurrency: 4 },
)
```

- **Lite**: spawns an isolated QuickJS runtime on a fresh OS thread, channels input/output as JSON.
- **Full**: wraps `Bun.Worker` via a data-URL worker script. Same API surface.

Task function must be self-contained (no closure captures). Arguments and return value marshaled as JSON.

---

## IPC architecture

| Channel | Direction | Transport |
|---|---|---|
| `api.<fn>()` | Frontend → Backend | wry `window.ipc.postMessage` → stdin JSON |
| `api.on()` | Backend → Frontend | Rust `evaluate_script` |
| `events.emit()` | Backend → Frontend | stdout JSON → Rust → `evaluate_script` |
| `dialog` / `fs` / `http` / … | Frontend → Rust | wry `window.ipc.postMessage` (direct, no backend round-trip) |
| `terminal:data` / `http:progress` | Rust → Frontend (stream) | tao user events → `evaluate_script` |

**No HTTP. No WebSocket. No firewall prompt.** Identical to Tauri v2's IPC stack.

Frontend assets served via `bv://localhost/` (wry custom protocol → filesystem). `window.location.origin` is `bv://localhost`.
