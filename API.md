# 📚 Tynd API Reference

Every Tynd app has three surfaces:

- **Backend** (`@tynd/core`) — imported by `backend/main.ts`, exposes `app.start`, emitters, lifecycle hooks.
- **Frontend RPC** (`@tynd/core/client`) — typed proxy to your backend functions (`createBackend<typeof backend>()`).
- **OS APIs** (`@tynd/core/client`) — direct bridge from the frontend to Rust: dialog, window, clipboard, shell, notification, tray, process, fs, http, websocket, sql, sidecar, terminal, store, os, path, compute.

> **Lite vs full parity:** all OS APIs live in Rust (`packages/host-rs/src/os/`) so both runtimes expose the exact same surface. See [`RUNTIMES.md`](./RUNTIMES.md) for JS-runtime differences.

---

## Table of contents

### [Backend API](#backend-api)
[`app.start(config)`](#appstartconfig) · [`AppConfig`](#appconfig) · [`WindowConfig`](#windowconfig) · [`createEmitter<T>()`](#createemittert) · [Native menu bar](#native-menu-bar) · [System tray](#system-tray) · [Streaming RPC](#streaming-rpc--async-generator-backend-handlers)

### [Frontend RPC — `createBackend<T>()`](#frontend-rpc--createbackendt)

### [OS APIs](#os-apis)
[`app`](#app--name--version--exit--relaunch) · [`dialog`](#dialog) · [`tyndWindow`](#tyndwindow) · [`menu`](#menu--react-to-menu-item-clicks) · [`clipboard`](#clipboard) · [`shell`](#shell) · [`notification`](#notification) · [`tray`](#tray) · [`process`](#process) · [`fs`](#fs) · [`shortcuts`](#shortcuts--system-wide-keyboard-hotkeys) · [`store`](#store) · [`os` / `path`](#os--path) · [`http`](#http) · [`websocket`](#websocket--full-duplex-client) · [`sql`](#sql--embedded-sqlite) · [`sidecar`](#sidecar--bundled-binaries) · [`terminal`](#terminal--real-pty-inside-the-app) · [`compute`](#compute--rust-native-cpu-helpers) · [`singleInstance`](#singleinstance--prevent-dual-launch) · [`updater`](#updater--auto-update-with-ed25519-verify) · [`workers`](#workers--offload-cpu-bound-js) · [Web-platform re-exports](#web-platform-re-exports) · [Binary IPC](#binary-ipc--tynd-bin)

### [IPC architecture](#ipc-architecture)

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

| Field         | Type            | Description                                                    |
| ------------- | --------------- | -------------------------------------------------------------- |
| `window`      | `WindowConfig`  | Window options                                                 |
| `frontendDir` | `string`        | Path to built frontend assets                                  |
| `devUrl`      | `string`        | Dev server URL (auto-detected; overrides `frontendDir` in dev) |
| `menu`        | `MenuSubmenu[]` | Native menu bar                                                |
| `tray`        | `TrayConfig`    | System tray                                                    |

### `WindowConfig`

| Field                    | Default | Description                 |
| ------------------------ | ------- | --------------------------- |
| `title`                  | `""`    | Window title                |
| `width`                  | `1200`  | Initial width               |
| `height`                 | `800`   | Initial height              |
| `minWidth` / `minHeight` | —       | Minimum size                |
| `maxWidth` / `maxHeight` | —       | Maximum size                |
| `resizable`              | `true`  | Allow resize                |
| `decorations`            | `true`  | Show title bar              |
| `transparent`            | `false` | Transparent background      |
| `alwaysOnTop`            | `false` | Pin above other windows     |
| `center`                 | `false` | Center on screen at startup |
| `fullscreen`             | `false` | Start fullscreen            |
| `maximized`              | `false` | Start maximized             |

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

### Streaming RPC — async-generator backend handlers

If a backend export is an `async function*`, the frontend gets a
`StreamCall<Y, R>` handle: awaitable (resolves to the generator's
`return` value) and async-iterable (yields each chunk). Cancellation
propagates end-to-end via `iterator.return()`.

```ts
// backend/main.ts
export async function* processFiles(paths: string[]) {
  let ok = 0
  for (const [i, path] of paths.entries()) {
    await doWork(path)
    ok++
    yield { path, progress: (i + 1) / paths.length }
  }
  return { ok, failed: paths.length - ok }
}

// frontend
const stream = api.processFiles(["a.txt", "b.txt"])
for await (const chunk of stream) render(chunk.progress)
const summary = await stream            // { ok, failed }
// or early-stop: await stream.cancel()
```

Marshalling: each `yield` sends one JSON line (full) or one
`__tynd_yield__` native call (lite); the final `return` value is
delivered as a single resolve on the promise side.

---

## OS APIs

Called from the frontend. They go through the Rust host directly — no round-trip to the TypeScript backend.

### `app` — name, version, exit, relaunch

```typescript
import { app } from "@tynd/core/client"

// Backend sets once at startup — typically from package.json fields.
await app.setInfo({ name: pkg.name, version: pkg.version })

// Anyone can query — frontend or backend.
const name    = await app.getName()     // fallback: binary file stem
const version = await app.getVersion()  // fallback: "0.0.0"

// Graceful lifecycle control.
await app.relaunch()   // spawn self + exit current
await app.exit(0)      // runs cleanup hooks first
```

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
await tyndWindow.setFocus()         // bring to front + keyboard focus
await tyndWindow.requestAttention() // flash taskbar / bounce Dock

const isMax  = await tyndWindow.isMaximized()
const isMin  = await tyndWindow.isMinimized()
const isFull = await tyndWindow.isFullscreen()
const isVis  = await tyndWindow.isVisible()

// Geometry — logical pixels, divide by scaleFactor() for device pixels.
await tyndWindow.setPosition(100, 100)
const pos  = await tyndWindow.getPosition()    // { x, y }
const size = await tyndWindow.getSize()         // inner { width, height }
await tyndWindow.setMinSize(400, 300)
await tyndWindow.setMaxSize(1920, 1080)
await tyndWindow.setResizable(false)
await tyndWindow.toggleMaximize()
const dpi = await tyndWindow.scaleFactor()      // 1.0 / 1.5 / 2.0
```

#### Monitor enumeration

```typescript
import { monitors } from "@tynd/core/client"

const all     = await monitors.all()       // Monitor[]
const primary = await monitors.primary()   // the OS's "main" display
const current = await monitors.current()   // where the primary window is
```

Each `Monitor` has `{ name, position, size, scale, isPrimary }`. Coordinates are physical pixels (divide `size` by `scale` for logical pixels).

#### Multi-window

The primary window has label `"main"`. Additional windows are created with a
unique label and get their own WebView + IPC channel; every `@tynd/core/client`
API call auto-targets the window it runs in (no label arg needed).

```typescript
import { tyndWindow } from "@tynd/core/client"

// Open a Settings window pointing at /settings in the same frontend.
await tyndWindow.create({
  label:  "settings",
  url:    "/settings",   // optional — defaults to the primary entry
  title:  "Settings",
  width:  600,
  height: 480,
})

const labels = await tyndWindow.all()  // ["main", "settings", ...]
await tyndWindow.close("settings")     // "main" cannot be closed this way

// Each window knows its own label:
console.log(tyndWindow.label())  // "main" or "settings" depending on caller
```

Window events (below) are broadcast to every webview and auto-filtered by
the current window's label — handlers only fire for their own window.

#### Window events

Subscribe handlers return an `unsubscribe()` function. Every handler is
identical on `lite` and `full` — events flow through the shared Rust
event emitter.

```typescript
import { tyndWindow } from "@tynd/core/client"

const offResize = tyndWindow.onResized(({ width, height }) => { /* … */ })
const offMove   = tyndWindow.onMoved(({ x, y }) => { /* … */ })
const offFocus  = tyndWindow.onFocused(() => { /* regained focus */ })
const offBlur   = tyndWindow.onBlurred(() => { /* lost focus */ })
const offTheme  = tyndWindow.onThemeChanged(({ theme }) => { /* "light" | "dark" */ })
const offDpi    = tyndWindow.onDpiChanged(({ scale }) => { /* e.g. 1.5 on 150% */ })

// State transitions — fire only on flip (not on initial state).
const offMin    = tyndWindow.onMinimized(() => {})
const offUnmin  = tyndWindow.onUnminimized(() => {})
const offMax    = tyndWindow.onMaximized(() => {})
const offUnmax  = tyndWindow.onUnmaximized(() => {})
const offFull   = tyndWindow.onFullscreen(() => {})
const offUnfull = tyndWindow.onUnfullscreen(() => {})

// Preventable close — call preventDefault() synchronously in the handler
// to cancel. The close proceeds after 500ms if nothing cancels it.
const offClose = tyndWindow.onCloseRequested((e) => {
  if (hasUnsavedChanges()) {
    e.preventDefault()
    void showSavePrompt()
  }
})

// Manual cancel (e.g. from a modal opened elsewhere during the 500ms window):
await tyndWindow.cancelClose()
```

### `menu` — react to menu item clicks

Menu bar items are declared in `tynd.config.ts`. This module lets the
frontend (or backend) subscribe to clicks. The handler fires when any
menu item (app menu bar or tray menu) with the given `id` is clicked.

```typescript
import { menu } from "@tynd/core/client"

const unsub = menu.onClick("file.new", () => newDocument())
// call unsub() to stop listening
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

// Watch for changes (ReadDirectoryChangesW / FSEvents / inotify).
const watcher = await fs.watch("./notes", { recursive: true }, (event) => {
  console.log(event.kind, event.path)
})
// Later: await watcher.unwatch()
```

### `autolaunch` — start the app at system boot

```typescript
import { autolaunch } from "@tynd/core/client"

await autolaunch.enable({ args: ["--minimized"] })
const on = await autolaunch.isEnabled()
await autolaunch.disable()
```

Platform-specific storage (Windows: `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`; macOS: `~/Library/LaunchAgents/<app>.plist`; Linux: `~/.config/autostart/<app>.desktop`). The registered path is whatever `std::env::current_exe()` resolves to — typically the installed binary, not the dev bin.

### `shortcuts` — system-wide keyboard hotkeys

```typescript
import { shortcuts, tyndWindow } from "@tynd/core/client"

const handle = await shortcuts.register("CmdOrCtrl+Shift+P", () => {
  tyndWindow.setFocus()
}, "open-palette")

await shortcuts.isRegistered("open-palette")  // true
await handle.unregister()
// Or bulk: await shortcuts.unregisterAll()
```

Accelerator strings use muda's format (`CmdOrCtrl+S`, `Alt+F4`, `Shift+Space`, …). Shortcuts fire even when the app is unfocused — Windows uses `RegisterHotKey`, macOS registers an Event Tap, Linux uses `XGrabKey` / portal.

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

### `websocket` — full-duplex client

```typescript
import { websocket } from "@tynd/core/client"

const ws = await websocket.connect("wss://echo.websocket.events")
ws.onOpen(() => ws.send("hello"))
ws.onMessage((msg) => {
  if (msg.kind === "text") console.log(msg.data)
  else console.log("binary bytes:", msg.data.byteLength)
})
ws.onClose(({ code }) => console.log("closed:", code))
ws.onError(({ message }) => console.error(message))

await ws.send("text frame")
await ws.send(new Uint8Array([1, 2, 3]))   // binary
await ws.ping()
await ws.close(1000, "bye")
```

Backed by `tungstenite` with rustls TLS (`wss://` out of the box). Each connection runs on a dedicated OS thread that polls the socket non-blocking and drains an outbound queue.

### `sql` — embedded SQLite

```typescript
import { sql } from "@tynd/core/client"

const db = await sql.open("./data.db")       // or ":memory:"
await db.exec("CREATE TABLE IF NOT EXISTS users(id INTEGER PRIMARY KEY, name TEXT)")

const { changes, lastInsertId } = await db.exec(
  "INSERT INTO users(name) VALUES (?1)",
  ["Alice"],
)

const rows = await db.query<{ id: number; name: string }>("SELECT * FROM users")
const one  = await db.queryOne<{ name: string }>("SELECT name FROM users WHERE id = ?1", [1])

await db.close()
```

Bundled SQLite via `rusqlite` — no system dependency. Params accept strings, numbers, booleans, nulls; arrays/objects are stored as JSON text. Blob columns come back as base64 strings.

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

// Hash raw bytes. Always returns a base64 string; hex is a one-liner
// convert in userland if you need it.
const digest = await compute.hash(bytes, { algo: "sha256" })
const blake  = await compute.hash(bytes)  // default algo = blake3

// OS CSPRNG, capped at 1 MiB per call.
const token = await compute.randomBytes(32)   // Uint8Array
```

Runs on a fresh Rust thread per call — never blocks the JS event loop. Works identically in lite and full. Supported `algo`: `blake3`, `sha256`, `sha384`, `sha512`.

For compression, use the standard `crypto.subtle` APIs or a pure-JS lib like `fflate` — see [ALTERNATIVES.md](ALTERNATIVES.md).

### `singleInstance` — prevent dual launch

```typescript
import { singleInstance } from "@tynd/core/client"

const { acquired } = await singleInstance.acquire("com.example.myapp")
if (!acquired) {
  // Duplicate launch — we've already forwarded argv+cwd to the primary,
  // which auto-focuses its window. Just exit silently.
  process.exit(0)
}

// In the primary instance: react to forwarded launches (e.g. deep links).
singleInstance.onSecondLaunch(({ argv, cwd }) => {
  console.log("reopened with", argv, "at", cwd)
})

// Dedicated handler for custom URL schemes declared in tynd.config.ts::protocols.
// Fires both on cold start (argv contains the URL) and on duplicate launch.
singleInstance.onOpenUrl((url) => {
  // e.g. "myapp://invite/abc123"
  router.navigate(new URL(url).pathname)
})
```

### Registering a URL scheme

Declare schemes in `tynd.config.ts` and `tynd build` wires each installer:

```typescript
// tynd.config.ts
export default {
  runtime: "lite",
  backend: "backend/main.ts",
  frontendDir: "dist",
  protocols: ["myapp"],   // myapp:// links now launch this app
  bundle:   { identifier: "com.example.myapp" },
} satisfies TyndConfig
```

- macOS `.app`: `CFBundleURLTypes` written into `Info.plist`.
- Windows NSIS/MSI: `HKCU\Software\Classes\<scheme>\shell\open\command` registry entries (currentUser — no admin prompt).
- Linux `.deb` / `.rpm` / `.AppImage`: `MimeType=x-scheme-handler/<scheme>;` + `%U` in the `Exec=` line of the generated `.desktop` file.

Reserved schemes (`http`, `https`, `file`, `ftp`, `mailto`, `javascript`, `data`, `about`, `blob`, `tynd`, `tynd-bin`) are rejected at config-validation time.

Backed by the `single-instance` crate for the OS lock (named pipe on Windows, abstract socket on Linux, CFMessagePort on macOS) plus `interprocess` for the local socket used to forward `{ argv, cwd }` from a duplicate launch to the primary. When `acquire()` returns `acquired: false`, the host has already:

1. Connected to the primary instance's socket over a platform-local channel.
2. Sent the forwarded payload as a single JSON line.
3. Triggered the primary window's `setFocus()` + un-minimize (via the host's native event loop, no IPC round-trip needed).

Use a stable reverse-DNS id — it doubles as the OS lock name and the socket name.

### `updater` — auto-update with Ed25519 verify

```typescript
import { updater } from "@tynd/core/client"

const info = await updater.check({
  endpoint:       "https://releases.example.com/update.json",
  currentVersion: "1.0.0",
})

if (info) {
  const off = updater.onProgress(({ phase, loaded, total }) => {
    console.log(`${phase}: ${loaded}/${total ?? "?"}`)
  })
  const { path } = await updater.downloadAndVerify({
    url:       info.url,
    signature: info.signature,
    pubKey:    "<base64 Ed25519 pubkey baked into the app>",
  })
  off()
  await updater.install({ path })  // swaps the binary + relaunches
}
```

Manifest shape (Tauri-compatible — any tooling that produces Tauri manifests works):

```json
{
  "version":  "1.2.3",
  "notes":    "Bug fixes & perf.",
  "pub_date": "2026-04-19T12:00:00Z",
  "platforms": {
    "windows-x86_64": { "url": "https://.../MyApp-1.2.3.exe", "signature": "<base64 Ed25519>" },
    "darwin-aarch64": { "url": "https://.../MyApp-1.2.3.dmg", "signature": "<base64 Ed25519>" },
    "linux-x86_64":   { "url": "https://.../MyApp-1.2.3.AppImage", "signature": "<base64 Ed25519>" }
  }
}
```

**Trust model.** The manifest itself is served over plain HTTPS (no meta-signature). Each platform entry carries an Ed25519 signature over the raw bytes of the downloaded file. The public key is supplied by the app — typically baked in at build time via `define:` or read from a config — so a compromised manifest server can only redirect to a URL whose bytes still have to verify against the local pubkey.

**Platform key.** Derived from `std::env::consts::OS` (with `macos` → `darwin` for parity with GitHub Releases / Tauri) and `std::env::consts::ARCH` (`x86_64` / `aarch64`). Manifest keys must use this `<os>-<arch>` form.

**Install semantics.** `install({ path, relaunch? = true })` swaps the downloaded binary for the running one and relaunches:
- **Windows**: runs `cmd /c timeout 2 & move /y <new> <current> & start <current>` — the timeout lets the current process exit so the `.exe` unlocks, then cmd replaces + relaunches. The call returns just before the current process exits.
- **Linux AppImage** (and any single-file binary): `fs::rename` + chmod +x + spawn + exit. Safe while running because Linux keeps the old inode mapped as long as the exe is live.
- **macOS**: not yet — `.app` bundles are directories and updates ship as archives; handle the swap manually for now.

#### Signing workflow

The CLI ships with a matching signer so you can produce manifests without extra tools:

```bash
# Generate a keypair once — commit only the .pub, keep the .key offline.
tynd keygen --out release/tynd-updater

# For each release, sign the freshly-built artifact. Output is base64 on stdout.
tynd sign release/MyApp-1.2.3.exe --key release/tynd-updater.key
# Or write it straight to a file:
tynd sign release/MyApp-1.2.3.exe --key release/tynd-updater.key \
  --out release/MyApp-1.2.3.exe.sig
```

Bake the `.pub` contents into your app's source so the runtime can verify:

```typescript
const UPDATER_PUB_KEY = "cFpG...RVDv/RQ="
await updater.downloadAndVerify({ url, signature, pubKey: UPDATER_PUB_KEY })
```

Under the hood the CLI uses the same WebCrypto Ed25519 primitives the runtime verifies with (`crypto.subtle.sign`) — signatures are raw 64-byte outputs, public keys are raw 32-byte exports, so the TS signer and the Rust `ed25519-dalek` verifier interop without format conversions.

**Not yet handled:**
- macOS `.app` swap (extract from `.dmg` / `.tar.gz`).
- Periodic auto-check.
- Delta updates.
- Rollback on failure.

### `workers` — offload CPU-bound JS

```typescript
import { workers } from "@tynd/core/client"

const w = await workers.spawn((input: number[]) => input.reduce((a, b) => a + b, 0))
const sum = await w.run<number, number[]>([1, 2, 3, 4, 5])
await w.terminate()
```

- **Lite**: spawns an isolated copy of the embedded JS engine on a fresh OS thread, channels input/output as JSON.
- **Full**: wraps `Bun.Worker` via a data-URL worker script. Same API surface.

Task function must be self-contained (no closure captures). Arguments and return value marshaled as JSON.

---

## IPC architecture

| Channel                           | Direction                 | Transport                                                    |
| --------------------------------- | ------------------------- | ------------------------------------------------------------ |
| `api.<fn>()`                      | Frontend -> Backend       | wry `window.ipc.postMessage` -> stdin JSON                   |
| `api.on()`                        | Backend -> Frontend       | Rust `evaluate_script`                                       |
| `events.emit()`                   | Backend -> Frontend       | stdout JSON -> Rust -> `evaluate_script`                     |
| `dialog` / `fs` / `http` / …      | Frontend -> Rust          | wry `window.ipc.postMessage` (direct, no backend round-trip) |
| `terminal:data` / `http:progress` | Rust -> Frontend (stream) | tao user events -> `evaluate_script`                         |

**No HTTP. No WebSocket. No firewall prompt.** Everything runs over the native wry bindings.

Frontend assets served via `tynd://localhost/` (wry custom protocol -> filesystem). `window.location.origin` is `tynd://localhost`.

### Web-platform re-exports

`@tynd/core/client` re-exports the standard Web globals as named exports
so `import * as tynd from "@tynd/core/client"` surfaces them in one
namespace alongside the Tynd OS APIs:

```ts
import * as tynd from "@tynd/core/client"

await tynd.fetch(url)
const ws = new tynd.WebSocket(wsUrl)
const hash = await tynd.crypto.subtle.digest("SHA-256", bytes)

// Framework APIs on the same namespace:
await tynd.fs.readText(path)
await tynd.sql.open(dbPath)
```

The exports are captured references to `globalThis.*`. In `lite` they
point at the Web-standard polyfills; in `full` they point at Bun's
native implementations — behavior matches the spec in both cases.

Available re-exports: `fetch`, `Request`, `Response`, `Headers`,
`AbortController`, `AbortSignal`, `ReadableStream`, `WebSocket`,
`EventSource`, `crypto`, `URL`, `URLSearchParams`, `TextEncoder`,
`TextDecoder`, `atob`, `btoa`, `Blob`, `File`, `FormData`,
`structuredClone`, `performance`.

### Binary IPC — `tynd-bin://`

Multi-MB payloads skip the JSON IPC. A second custom protocol, `tynd-bin://localhost/<api>/<method>?<query>`, carries raw bytes in the request and response bodies — no base64, no JSON envelope, `ArrayBuffer` on arrival. Current routes:

| Route | Method | In | Out |
|---|---|---|---|
| `fs/readBinary?path=...` | `GET` | — | file bytes |
| `fs/writeBinary?path=...&createDirs=0\|1` | `POST` | bytes | `204` |
| `compute/hash?algo=blake3\|sha256\|sha384\|sha512&encoding=base64` | `POST` | bytes | UTF-8 digest |

The TS client wraps these: `fs.readBinary(path)`, `compute.hash(bytes)`, etc. — users don't interact with the scheme directly. Small / non-binary calls (`randomBytes`, text helpers, terminal events) stay on the JSON IPC where it's simpler.
