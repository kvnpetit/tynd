# Getting started with Tynd

Build a native desktop app in TypeScript in under 5 minutes.

## Prerequisites

- **[Bun](https://bun.sh)** — `curl -fsSL https://bun.sh/install | bash`
- **A C compiler toolchain** — needed if `@tynd/host` can't find a pre-built binary for your OS/arch. On macOS it's Xcode CLT, on Linux it's `build-essential`, on Windows it's MSVC build tools.

Linux only — WebView deps:

```bash
sudo apt-get install -y \
  libgtk-3-dev libwebkit2gtk-4.1-dev \
  libjavascriptcoregtk-4.1-dev libsoup-3.0-dev
```

## Scaffold a project

```bash
bunx @tynd/cli create my-app
```

The CLI asks for:
- **Project name** (defaults to `my-app`)
- **Frontend framework** — React, Vue, Svelte, Solid, Preact, Lit, or Angular
- **Runtime** — `full` (recommended, full Bun API) or `lite` (smaller binary, ~5 MB)

Then scaffolds the frontend via Vite / Angular CLI and drops a Tynd config on top.

```bash
cd my-app
tynd dev
```

A native window opens with your frontend + Vite HMR. Save `backend/main.ts` — hot reloads without tearing down the window.

## Project layout

```
my-app/
├── tynd.config.ts       ← runtime + paths
├── package.json
├── backend/
│   └── main.ts          ← backend entry (app.start + exported functions)
└── src/                 ← frontend source (React / Vue / …)
    └── main.tsx
```

## Backend — your TypeScript functions

```ts
// backend/main.ts
import { app, createEmitter } from "@tynd/core"

export const events = createEmitter<{
  userCreated: { id: string; name: string }
}>()

export async function greet(name: string): Promise<string> {
  return `Hello, ${name}!`
}

app.onReady(() => {
  events.emit("userCreated", { id: "1", name: "Alice" })
})

app.start({
  window: {
    title: "My App",
    width: 1200,
    height: 800,
    center: true,
  },
})
```

Every exported function and emitter is **automatically callable from the frontend** — no codegen, no glue.

## Frontend — typed client

```tsx
// src/App.tsx
import { createBackend } from "@tynd/core/client"
import type * as backend from "../backend/main"

const api = createBackend<typeof backend>()

export default function App() {
  const [msg, setMsg] = useState("")

  useEffect(() => {
    api.greet("world").then(setMsg)
    api.on("userCreated", (u) => console.log("new user:", u.name))
  }, [])

  return <h1>{msg}</h1>
}
```

Types come from `typeof backend`. Rename a function in the backend → compiler catches the frontend call.

## Build a production binary

```bash
tynd build
```

Output: a single self-contained `.exe` / binary under `release/`.

- **full** runtime: ~40 MB (Bun bundled)
- **lite** runtime: ~5 MB (QuickJS embedded)

Ship that file. The user double-clicks it. No installer, no framework to install, no Node/Bun on their machine.

### Platform installers

Need a real installer (icon in Applications, Start Menu entry, apt/dnf compatible)? Add a `bundle` block to `tynd.config.ts`:

```ts
// tynd.config.ts
import type { TyndConfig } from "@tynd/cli"

export default {
  runtime: "full",
  backend: "backend/main.ts",
  frontendDir: "dist",
  bundle: {
    identifier: "com.yourco.myapp",   // reverse-DNS, required
    categories: ["Utility"],          // optional — XDG / Launch Services
    shortDescription: "A tiny app",
  },
} satisfies TyndConfig
```

Then build with `--bundle`:

```bash
tynd build --bundle              # all formats applicable to your host OS
tynd build --bundle app,dmg      # specific formats
```

| On… | You get |
|---|---|
| macOS | `MyApp.app` (double-click bundle) + `MyApp-1.0.0.dmg` (draggable installer) |
| Linux | `.deb` (Debian/Ubuntu) + `.rpm` (Fedora/RHEL, requires `rpmbuild`) + `.AppImage` (portable) |
| Windows | `MyApp-1.0.0-setup.exe` (NSIS wizard) + `MyApp-1.0.0-x64.msi` (MSI) |

Build tools (NSIS, WiX v3, appimagetool) are auto-downloaded once into `.tynd/cache/tools/` — no manual install. `rpmbuild` is the only exception: install it via `sudo apt install rpm` or `sudo dnf install rpm-build` on the build machine.

Cross-compilation isn't supported — each host produces installers only for its own OS. Use GitHub Actions with a matrix to cover all three (see `.github/workflows/build-host.yml` in the Tynd repo for a reference).

## Native OS APIs from the frontend

Tynd exposes dialogs, window control, clipboard, shell, notifications, and tray **directly from the frontend** — no IPC round-trip through your backend:

```ts
import { dialog, tyndWindow, clipboard, notification } from "@tynd/core/client"

const file = await dialog.openFile({
  title: "Open image",
  filters: [{ name: "Images", extensions: ["png", "jpg"] }],
})

await tyndWindow.setTitle("My App — Unsaved")
await clipboard.writeText("hello")
await notification.send("Build done", { body: "0 errors" })
```

## What's next

- **[RUNTIMES.md](./RUNTIMES.md)** — `lite` vs `full` detailed comparison (APIs, perf, compat)
- **[COMPARISON.md](./COMPARISON.md)** — Tynd vs Tauri / Wails / Electron across 38 categories
- **[playground/](./playground/)** — working React examples in both runtimes

## Common commands

```bash
tynd dev                  # dev mode with HMR
tynd build                # production binary
tynd build --bundle       # + platform installers (app/dmg, deb/rpm/AppImage, exe/msi)
tynd validate             # check config + file structure
tynd clean [--dry-run]    # remove build artifacts
tynd info [--json]        # environment + project info
tynd upgrade              # bump @tynd/* deps to latest
tynd --verbose <cmd>      # debug-level logs
```

## Stuck?

- Run `tynd info --verbose` to see what Tynd sees about your environment
- Run `tynd validate` to check your config + binary availability
- Open an [issue](https://github.com/kvnpetit/tynd/issues) with the output
