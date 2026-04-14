# bunview

Native desktop apps with **Bun + TypeScript** — same concept as Tauri/Wails but 100% TS.

| | Tauri v2 | Wails v3 | **bunview** |
|---|---|---|---|
| Backend language | Rust | Go | **TypeScript / Bun** |
| Frontend | Any | Any | Any |
| Typed RPC | Rust → TS codegen | Go → TS codegen | **zero codegen** — `typeof commands` |
| Webview | native OS (wry) | native OS | **native OS (wry + tao)** |
| Native IPC | `webview_bind` | HTTP/WS | **`webview_bind` + `evaluate_script`** |
| Hardware introspection | ❌ | ❌ | **17 APIs (CPU/GPU/RAM/NPU/…)** |
| WebGPU + WebNN toggle | ❌ | ❌ | **opt-in `hardwareAcceleration`** |
| NPU detection | ❌ | ❌ | **Intel/AMD/Apple/Qualcomm** |

---

## How it works

```
Your Bun backend                            Native OS window
────────────────────────                    ─────────────────────────
  commands module    ◄── native IPC ──────  await client.rpc.cmd(payload)
  app.emit(...)      ───── push ─────────►  client.listen("event", cb)
                          (wry webview_bind,
                           zero TCP, zero WebSocket)
                     ↕ stdin/stdout JSON
                     webview-host  (Rust subprocess, wry + tao)
```

**Zero network stack.** Frontend assets served via `bv://` custom protocol (wry `with_custom_protocol`), RPC via native `webview_bind`, events via `webview_eval`. Same architecture as Tauri v2.

---

## Quick start

```bash
git clone <repo>
cd bunview
bun install          # downloads the pre-compiled webview-host for your platform

cd examples/hello-world
bun run dev          # opens native window in watch mode
```

---

## CLI

```bash
bunview build                  # compile for current platform (auto-detected)
bunview build --windows        # windows-x64
bunview build --windows-x64    # windows-x64  (Intel/AMD)
bunview build --windows-arm64  # windows-arm64 (Surface, Snapdragon)
bunview build --linux          # linux-x64
bunview build --linux-x64      # linux-x64    (Intel/AMD)
bunview build --linux-arm64    # linux-arm64  (Raspberry Pi, AWS Graviton)
bunview build --macos          # auto (x64 or arm64)
bunview build --macos-x64      # macos-x64    (Intel Mac)
bunview build --macos-arm64    # macos-arm64  (Apple Silicon M1/M2/M3/M4)

bunview dev                    # dev mode with HMR
bunview start                  # run the built app
bunview --help                 # show help
```

### Cross-compilation matrix

> ✅ Full — standalone binary, ready to run  
> 🔶 Partial — raw binary only, no `.app`/`.dmg`, no code signing  
> ❌ Impossible

| Build machine → Target | Windows `.exe` | Linux | macOS (raw binary) | macOS `.app` / `.dmg` |
|---|---|---|---|---|
| **Windows** | ✅ | ✅ | 🔶 | ❌ |
| **Linux** | ✅ | ✅ | 🔶 | ❌ |
| **macOS** | ✅ | ✅ | ✅ | ✅ |

**🔶 Raw macOS binary**: the Mach-O executable works on macOS when run from the terminal, but it is not packaged as a `.app` bundle, not signed, and Gatekeeper will block it for end users.

**For proper macOS distribution** (`.app`, `.dmg`, notarization) you need a macOS machine — this is an Apple constraint, not a bunview one.

**No prerequisites needed** — no C++ toolchain, no cross-compiler, nothing to install.

---

## Build output

`bunview build` produces a self-contained `dist/` folder:

```
dist/
  my-app.exe            ← standalone Bun binary (no Bun runtime needed)
  frontend/             ← static assets (HTML, JS, CSS)
  bin/
    windows-x64/
      webview-host.exe  ← native webview subprocess
```

Ship the entire `dist/` folder — nothing else required on the end-user machine (except the OS WebView runtime, which is pre-installed on all modern systems).

| OS | WebView runtime | Pre-installed? |
|---|---|---|
| Windows 10/11 | WebView2 (Edge) | ✅ yes |
| macOS | WKWebView | ✅ yes |
| Linux | WebKit2GTK | ⚠️ `sudo apt install libwebkit2gtk-4.1-0` |

### Installer artifacts generated per OS

Each `bunview build` produces a **portable** artifact (run anywhere, no install) and an **installer** artifact (with system integration). The installer step fans out to multiple formats where relevant:

| OS | Portable | Installer(s) | External tool required |
|---|---|---|---|
| **Windows** | NSIS SFX `.exe` | `-setup.exe` (NSIS) **+** `.msi` (WiX v3) | none — NSIS + WiX auto-downloaded on first build |
| **macOS** | `.app` bundle | `.dmg` disk image | `hdiutil` (bundled with macOS) |
| **Linux** | `.AppImage` | `.deb` (Debian/Ubuntu) **+** `.rpm` (Fedora/RHEL) | none — `appimagetool` + `nfpm` auto-downloaded |

**Zero-install build philosophy** — every OS can produce every format native to it without the developer installing anything beyond Bun. Build-time tools (NSIS, WiX v3, appimagetool, nfpm) are downloaded on demand into `host/.deps/` (gitignored) and cached across builds. NSIS is the consumer-friendly Windows installer; MSI is what enterprise IT wants for Group Policy / SCCM / Intune. `.deb` + `.rpm` together cover ~85% of desktop Linux users.

---

## Publishing releases

Binaries for all platforms are built automatically by GitHub Actions when you push a version tag:

```bash
git tag v1.0.0
git push origin v1.0.0
```

The CI workflow (`.github/workflows/build-host.yml`) builds the `webview-host` on 5 runners (Windows, Linux x64, Linux arm64, macOS x64, macOS arm64) and attaches them to the release.

### Single instance & deep linking

Activate `singleInstance: true` in `createApp()` — 2nd launches are forwarded to the primary (window focused + `onSecondInstance(cb)` callback receives the new argv). Combine with `urlScheme` for deep-link handling:

```typescript
// backend/main.ts
const app = createApp({
  entry:    "./frontend",
  commands,
  singleInstance: true,
  urlScheme: "myapp",          // enables myapp:// URL handling
});

app.onSecondInstance(({ argv, cwd }) => {
  console.log("2nd launch args:", argv);
});

app.onDeepLink((url) => {
  console.log("Deep link:", url);   // e.g. "myapp://auth/callback?code=…"
});
```

```typescript
// bunview.config.ts — registers the scheme in installer / .app / .desktop
export default defineConfig({
  entry: "backend/main.ts",
  urlScheme: "myapp",
});
```

Scheme registration is automatic:
- **Windows** — NSIS installer adds HKCU\Software\Classes\myapp registry keys
- **macOS** — Info.plist gets a `CFBundleURLTypes` entry
- **Linux** — `.desktop` file gets `MimeType=x-scheme-handler/myapp;`

### Code signing

Add a `codeSigning` block to `bunview.config.ts`. String values support `${ENV_VAR}` interpolation.

```typescript
import { defineConfig } from "bunview";

export default defineConfig({
  entry:    "backend/main.ts",
  frontend: "./frontend",
  name:     "my-app",
  codeSigning: {
    windows: {
      certificate:  "./cert.pfx",
      password:     "${WINDOWS_CERT_PASSWORD}",
      timestampUrl: "http://timestamp.digicert.com",  // default
      description:  "My App",
    },
    macos: {
      identity:        "Developer ID Application: Your Name (TEAMID)",
      hardenedRuntime: true,   // required for notarization
      entitlements:    "./entitlements.plist",
      notarize: {
        appleId:  "${APPLE_ID}",
        teamId:   "${APPLE_TEAM_ID}",
        password: "${APPLE_APP_PASSWORD}",  // app-specific password
      },
    },
  },
});
```

Signing runs automatically during `bunview build` — main binary, portable, and installer/.dmg are all signed. Notarization is applied to the final distribution artifact (`.dmg` on macOS, installer `.exe` on Windows not applicable).

**Tools required**:
- Windows: `signtool.exe` (from Windows SDK, auto-on-PATH on Windows 10+)
- macOS: `codesign` + `xcrun notarytool` + `xcrun stapler` (bundled with Xcode Command Line Tools)
- Linux: no signing (rpm/deb signing can be added via external tooling)

#### Windows SmartScreen & certificate types

Windows Defender SmartScreen flags every unfamiliar `.exe` on first launch. To avoid the "Windows protected your PC" dialog, the options in order of effectiveness:

| Cert type | Cost/year | SmartScreen behavior |
|---|---|---|
| **EV Code Signing** (hardware token) | ~$300–600 | ✅ Instant reputation — no warning from first download |
| **Standard OV Code Signing** | ~$100–200 | ⚠️ Warning shown until Microsoft builds reputation (hundreds of installs) |
| **Self-signed / unsigned** | free | ❌ Warning shown forever, portable `.exe` will look broken to end users |

Recommendation for anything user-facing: **use an EV cert**. The signing config above works with both EV (hardware token; `signtool` reads it transparently via the Windows cert store) and standard `.pfx` files. For EV, leave `certificate` pointing at an empty path or use `signtool`'s `/n "Subject Name"` by customizing `cli/utils/sign.ts` if needed.

For internal tools where users are expected to bypass SmartScreen manually, standard OV signing is fine.

---

## Creating a project

```bash
bunview init my-app
cd my-app
bun install
bun run dev
```

Or manually:

### 1. `bunview.config.ts`

```typescript
import { defineConfig } from "bunview";

export default defineConfig({
  entry:    "backend/main.ts",   // Bun backend entry point
  frontend: "./frontend",    // static assets directory — copied to dist/
  name:     "my-app",        // output binary name (default: package.json name)
  outDir:   "dist",          // output directory (default: "dist")
});
```

### 2. `package.json` scripts

```json
{
  "scripts": {
    "dev":   "bunview dev",
    "start": "bunview start",
    "build": "bunview build"
  },
  "dependencies": {
    "bunview": "^0.1.0"
  }
}
```

### 3. Backend — commands as plain async functions

Every function exported from `commands.ts` is callable from the frontend. Keep backend-only helpers in a separate module (e.g. `internal.ts`) and they are unreachable from the webview by construction — no config, no allowlist to maintain.

```typescript
// backend/commands.ts — exposed to frontend via client.rpc.*
export async function greet(name: string) {
  return `Hello, ${name}!`;
}
```

```typescript
// backend/internal.ts — backend-only, never passed to createApp
export async function cleanup() { /* flush caches, close DB, … */ }
```

```typescript
// backend/main.ts
import { createApp } from "bunview";
import * as commands from "./commands";
import { cleanup } from "./internal";

export type AppCommands = typeof commands;

const app = createApp({
  entry: "./frontend",
  commands,
  window: { title: "My App", width: 1024, height: 768 },
});

app.onReady(() => console.log("Window ready"));
app.onClose(() => { cleanup(); console.log("Window closed"); });

await app.run();
```

> **Security model** — the IPC exposure surface is *exactly* the `commands` module. There is no `permissions` list, no ACL config. Move a function between `commands.ts` and `internal.ts` to flip its visibility. Code and policy cannot drift because they are the same thing.

### 4. Frontend — typed RPC proxy

```typescript
import type { AppCommands } from "../backend/main";
import { createClient } from "bunview/client";

const client = createClient<AppCommands>({ timeout: 10_000 });

// Direct, fully-typed call via .rpc
const msg = await client.rpc.greet("World"); // string

// Events
client.listen("tick", (ts) => console.log("tick", ts));
client.once("ready", () => console.log("first ready event"));
client.onConnectionChange((ok) => console.log(ok ? "connected" : "lost"));
```

---

## API Reference

### Backend — `createApp()`

```typescript
const app = createApp({
  entry:    "./frontend",           // frontend assets directory
  commands: myCommands,             // imported module of async functions
  window: {
    title:     "My App",            // window title
    width:     900,                 // initial width
    height:    600,                 // initial height
    resizable: true,                // set false to lock size
    debug:     false,               // open devtools
  },
  port: 0,                          // 0 = random
});
```

#### Lifecycle

| Method | Description |
|---|---|
| `app.run()` | Start the app (blocks until window closes) |
| `app.onReady(cb)` | Called when webview is ready |
| `app.onClose(cb)` | Called when window closes (cleanup) |

#### Window

| Method | Description |
|---|---|
| `app.window.setTitle(title)` | Change window title |
| `app.window.setSize(w, h)` | Resize window |
| `app.window.minimize()` | Minimize to taskbar/dock |
| `app.window.maximize()` | Maximize window |
| `app.window.restore()` | Restore from minimize/maximize |
| `app.window.fullscreen(enter?)` | Toggle fullscreen (default: `true`) |
| `app.window.center()` | Center on screen |
| `app.window.setMinSize(w, h)` | Set minimum size |
| `app.window.setMaxSize(w, h)` | Set maximum size |
| `app.window.setAlwaysOnTop(on)` | Pin above other windows |
| `app.window.hide()` | Hide window |
| `app.window.show()` | Show window |
| `app.window.navigate(url)` | Navigate to URL |
| `app.window.close()` | Close window |

#### Other

| Method | Description |
|---|---|
| `app.eval(code)` | Execute JavaScript in the webview |
| `app.emit(event, payload)` | Push typed event to frontend |
| `app.exit(code?)` | Clean shutdown, closes window + exits process |
| `app.relaunch()` | Spawn a fresh instance then exit (post-update restart) |

#### Paths (cross-platform, auto-create)

XDG-compliant on Linux, Apple HIG on macOS, Known Folders on Windows.

| Method | Windows | macOS | Linux |
|---|---|---|---|
| `app.paths.data()` | `%APPDATA%\<App>` | `~/Library/Application Support/<App>` | `~/.local/share/<App>` |
| `app.paths.config()` | same as data | `~/Library/Preferences/<App>` | `~/.config/<App>` |
| `app.paths.cache()` | `%LOCALAPPDATA%\<App>\Cache` | `~/Library/Caches/<App>` | `~/.cache/<App>` |
| `app.paths.logs()` | `data/logs` | `~/Library/Logs/<App>` | `~/.local/state/<App>/logs` |
| `app.paths.temp()` | OS temp | OS temp | OS temp |
| `app.paths.home()` / `.executable()` | user home / `process.execPath` | idem | idem |
| `app.paths.downloads()` / `documents()` / `desktop()` / `pictures()` / `music()` / `videos()` | standard user dirs | idem | idem |

#### OS introspection (`app.os`)

| Member | Returns |
|---|---|
| `.platform` | `"windows" \| "macos" \| "linux"` |
| `.arch` | `"x64" \| "arm64" \| …` |
| `.family` | `"windows" \| "unix"` |
| `.eol` | `"\n"` or `"\r\n"` |
| `.version()` | kernel/OS release |
| `.locale()` | BCP-47 (`"en-US"`, `"fr-FR"`…) |
| `.hostname()` / `.uptime()` / `.totalMemory()` / `.freeMemory()` / `.userInfo()` | system info |

#### Persistent key-value store (`app.store`)

Atomic JSON persisted at `paths.data()/store.json`.

```typescript
app.store.set("theme", "dark");
const t = app.store.get<string>("theme");      // "dark"
app.store.has("theme");                         // true
app.store.delete("theme");
app.store.entries();                            // [[key, value], …]
app.store.clear();
```

#### Structured logging (`app.log`)

Rotating file logger at `paths.logs()/app.log` (5 MB × 3 rotations by default).

```typescript
app.log.info("app ready", { version: "1.0.0" });
app.log.warn("deprecated call");
app.log.error("crash", { stack: err.stack });

// Custom level / directory:
app.createLogger({ level: "debug", dir: "/var/log/myapp", maxSize: 10_000_000 });
```

#### File watcher (`app.watch`)

```typescript
const unwatch = app.watch("/path/to/dir", (event) => {
  console.log(event.type, event.path);  // "change" | "rename"
}, { recursive: true });
// unwatch(); to stop
```

#### CLI arguments (`app.cliArgs`)

```bash
myapp --debug --port=3000 --name Alice file.txt
```
```typescript
app.cliArgs.flags;        // { debug: true, port: "3000", name: "Alice" }
app.cliArgs.positionals;  // ["file.txt"]
```

#### Binary data in IPC

`Uint8Array` / `ArrayBuffer` / `Buffer` are auto-encoded (base64) transparently.

```typescript
// backend
export async function processImage(bytes: Uint8Array): Promise<Uint8Array> {
  return sharp(bytes).resize(400).toBuffer();
}
```
```typescript
// frontend
const file = await (await fetch("/pic.jpg")).arrayBuffer();
const resized = await client.rpc.processImage(new Uint8Array(file));
//    ^? Uint8Array
```

> **Note**: base64 adds ~33% overhead. For payloads > 10 MB, prefer file-path IPC.

#### HTTP download with progress

```typescript
import { downloadFile } from "bunview/http";

await downloadFile("https://…/update.zip", {
  dest: `${app.paths.cache()}/update.zip`,
  onProgress: ({ percent, loaded, total }) => {
    if (percent !== null) app.window.setProgressBar(percent / 100);
  },
  signal: AbortSignal.timeout(60_000),
});
```

#### Taskbar / Dock integration

| Method | Windows | macOS | Linux |
|---|---|---|---|
| `app.window.setProgressBar(0.42)` | taskbar progress | dock progress | ❌ |
| `app.window.setBadgeCount(5)` | ❌ | dock tile badge | ❌ |
| `app.window.setBadgeCount(null)` | — | clear badge | — |

#### Hardware & AI

Full native hardware introspection — off by default, enabled per-command:

```typescript
const info = await app.hardware.getSystemInfo();       // CPU, RAM, GPU, OS
const gpus = await app.hardware.getGpuUsage();          // util%, VRAM, temp, power
const caps = await app.hardware.getAiCapabilities();    // CUDA, ROCm, Vulkan, Metal, DirectML, WebNN, NPU
const ram  = await app.hardware.getRamDetails();        // DDR4/DDR5, speed, manufacturer

app.hardware.startMonitoring(1000);                     // emits `hwMonitorUpdate` events
app.hardware.onUpdate((data) => console.log(data.cpu.global));
```

Available via `app.hardware.*`: `getSystemInfo`, `getCpuUsage`, `getCpuDetails`, `getMemoryInfo`, `getRamDetails`, `getBatteryInfo`, `getDiskInfo`, `getNetworkInfo`, `getNetworkSpeed`, `getGpuUsage`, `getTemperature`, `getUsbDevices`, `getAudioDevices`, `getDisplayInfo`, `getProcessList`, `getUsers`, `getAiCapabilities`, `startMonitoring` / `stopMonitoring` / `onUpdate`.

#### WebGPU & WebNN in the WebView

```typescript
createApp({
  window: { hardwareAcceleration: true },   // default: false
});
```

Enables WebGPU compute + WebNN (neural network API) in the WebView — routing inference to GPU + NPU via DirectML (Windows) or CoreML/ANE (Apple Silicon). Required for Transformers.js WebGPU backend and on-device ML models.

### Frontend — `createClient()`

```typescript
const client = createClient<AppCommands, AppEvents>({
  timeout: 30_000,   // invoke timeout in ms (0 = no timeout)
});
```

| Member | Description |
|---|---|
| `client.rpc.<cmd>(arg)` | Typed proxy — each backend command appears as a method |
| `client.invoke(cmd, arg)` | Dynamic dispatch by string name (fallback) |
| `client.listen(event, handler)` | Subscribe to event, returns `unlisten()` |
| `client.once(event, handler)` | Listen once, auto-unsubscribes |
| `client.emit(event, payload)` | Push event from frontend to backend |
| `client.isConnected` | `boolean` |
| `client.onConnectionChange(cb)` | Subscribe to connection changes |

---

## IPC — how it works

| Channel | Direction | Transport |
|---|---|---|
| `client.rpc.*` | Frontend → Backend | wry `window.ipc.postMessage` → stdin JSON |
| `client.listen()` | Backend → Frontend | `webview.evaluate_script` (native eval push) |
| `app.emit()` | Backend → Frontend | same `evaluate_script` pipeline |
| `client.emit()` | Frontend → Backend | `window.__bv_emit__` → stdin JSON |

All RPC Promises resolve natively via `webview_resolve`. **No HTTP, no WebSocket, no firewall prompt.**

### Streaming results with `Channel<T>`

For commands that need to push progress updates (long downloads, training jobs, tail-like streams), use a typed `Channel` instead of polling:

```typescript
// frontend
import { Channel, createClient } from "bunview/client";

const client = createClient<AppCommands>();
const progress = new Channel<{ done: number; total: number }>();
progress.onMessage((msg) => updateBar(msg.done / msg.total));

await client.rpc.importCsv({ path: "/big.csv", progress });
progress.close();
```

```typescript
// backend — commands.ts
import { Channel } from "bunview";

export async function importCsv(args: {
  path: string;
  progress: Channel<{ done: number; total: number }>;
}) {
  const total = /* … */ 1000;
  for (let done = 0; done < total; done++) {
    /* … */
    args.progress.send({ done, total });
  }
}
```

The Channel auto-generates a unique event id — no bookkeeping needed. Values travel over the same `evaluate_script` pipe as `client.listen()`.

### Multi-window

```typescript
const prefs = await app.createWindow({
  url: "bv://localhost/preferences.html",
  title: "Preferences",
  width: 600,
  height: 400,
  resizable: false,
});

prefs.onClose(() => console.log("prefs closed"));
prefs.emit("settings:loaded", currentSettings);
```

Each auxiliary window is its own OS process sharing the backend `commands` — IPC works identically from every window. `app.windows` lists all currently open auxiliary windows.

### Lazy commands

For apps with large command modules, defer loading until the window is ready:

```typescript
const app = createApp({
  commands: () => import("./commands"),   // imported only after bunview boots
  window: { title: "My App" },
});
```

The thunk can be sync or async; bunview awaits it once before wiring IPC.

---

## Project structure

```
bunview/
├── .github/workflows/
│   └── build-host.yml        ← CI: builds webview-host for all platforms on tag push
├── packages/bunview/
│   ├── cli/
│   │   └── index.ts          ← bunview CLI (build / dev / run)
│   ├── src/
│   │   ├── index.ts          ← Public API exports
│   │   ├── app.ts            ← createApp(), BunviewApp class + hardware getter
│   │   ├── server.ts         ← HTTP asset server (dev mode only)
│   │   ├── host.ts           ← Native webview subprocess manager
│   │   ├── client-script.ts  ← JS bridge injected into every page
│   │   └── types.ts          ← TypeScript types & IPC protocol
│   ├── client/
│   │   ├── index.ts          ← createClient<T>() frontend SDK with .rpc proxy
│   │   └── types.ts          ← BunviewClient, RpcProxy types
│   ├── host-rs/              ← Rust native host (wry + tao)
│   │   ├── Cargo.toml        ← deps: wry, tao, sysinfo, starship-battery, nusb, if-addrs
│   │   └── src/
│   │       ├── main.rs       ← entry + event loop + browser args
│   │       ├── ipc.rs        ← JS shim + IPC encoding
│   │       ├── scheme.rs     ← bv:// custom protocol handler
│   │       └── cmds/
│   │           ├── hardware/ ← 7 submodules (cpu/gpu/ai/network/devices/monitor/system)
│   │           ├── window.rs / appearance.rs / dialog.rs / menu.rs / tray.rs
│   │           └── mod.rs    ← dispatch
│   ├── scripts/
│   │   ├── build-host.ts     ← cargo build --release wrapper
│   │   └── setup.ts          ← postinstall: downloads webview-host from GitHub Releases
│   └── bin/                  ← downloaded binaries (gitignored)
│       └── {platform-arch}/webview-host(.exe)
└── examples/
    ├── hello-world/
    │   ├── bunview.config.ts ← { entry, frontend, name }
    │   ├── backend/
    │   │   ├── main.ts       ← createApp() + lifecycle
    │   │   ├── commands.ts   ← exposed to frontend via client.rpc.*
    │   │   └── internal.ts   ← backend-only helpers (not exposed)
    │   └── frontend/         ← static HTML + TypeScript UI
    └── react-app/
        ├── bunview.config.ts
        ├── backend/
        │   ├── main.ts
        │   ├── commands.ts   ← exposed commands
        │   └── internal.ts   ← backend-only
        ├── src/              ← React frontend source (Vite)
        └── frontend/         ← Vite build output (served by bunview)
```

---

## Environment variables

| Variable | Effect |
|---|---|
| `BUNVIEW_SKIP_BUILD=1` | Skip postinstall entirely (used by CI) |
| `BUNVIEW_BUILD_FROM_SOURCE=1` | Build `webview-host` from Rust source instead of downloading |

Building from source requires a Rust toolchain:

| Platform | Requirement |
|---|---|
| All | [rustup](https://rustup.rs) (stable toolchain, 2021 edition) |
| Linux | `sudo apt install libgtk-3-dev libwebkit2gtk-4.1-dev` (for WebKitGTK) |
| Windows / macOS | No additional native deps — wry bundles everything |
