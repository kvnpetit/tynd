# @tynd/core

The backend API and frontend client for [Tynd](https://github.com/kvnpetit/tynd) — desktop apps in TypeScript.

## Prerequisites

**[Bun](https://bun.sh) is required** during development. Every Tynd backend runs on Bun; Node.js is not a supported substitute. Install Bun once:

```bash
# macOS / Linux / WSL
curl -fsSL https://bun.sh/install | bash

# Windows (PowerShell)
powershell -c "irm bun.sh/install.ps1 | iex"
```

**[Rust](https://rustup.rs) 1.95 or newer** is also required, with the platform build tools: `tynd dev` and `tynd build` compile each app's native host with Cargo. You write no Rust. See [Installation](https://kvnpetit.github.io/tynd/docs/v1.0/getting-started/installation).

```bash
# macOS / Linux
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Windows: run rustup-init.exe from https://rustup.rs
```

End users of built apps need nothing — the runtime ships inside the binary.

## Install

```bash
bun add @tynd/core
bun add -d @tynd/cli @tynd/host-src
```

## Two entrypoints

```ts
// backend/main.ts  (Bun process)
import { app, createEmitter } from "@tynd/core"

// src/App.tsx  (WebView)
import { createBackend, tyndWindow /* ...and more core client APIs */ } from "@tynd/core/client"

// Plugins bring their own import path. Only the plugins your app declares
// are compiled into its host.
import { terminal } from "@tynd/plugin-terminal/client"
```

## Backend — `app.start()` + typed emitters

```ts
import { app, createEmitter } from "@tynd/core"

export const events = createEmitter<{ ready: { message: string } }>()

export async function greet(name: string): Promise<string> {
  return `Hello, ${name}!`
}

app.onReady(() => events.emit("ready", { message: "app is up" }))

app.start({
  frontendDir: import.meta.dir + "/../dist",
  window: { title: "My App", width: 1200, height: 800, center: true },
})
```

Every exported function becomes callable from the frontend. No codegen.

## Frontend — zero-codegen typed RPC

```ts
import { createBackend } from "@tynd/core/client"
import { dialog } from "@tynd/plugin-dialog/client"
import { createStore } from "@tynd/plugin-store/client"
import type * as backend from "../backend/main"

const store = createStore("app")
const api = createBackend<typeof backend>()
const msg = await api.greet("Alice")   // string ✅ — types flow from backend
api.on("ready", ({ message }) => console.log(message))

// Direct OS APIs from official plugins — bypass the backend
const file = await dialog.openFile()
await store.set("recent-file", file)
```

## Core OS APIs (`@tynd/core/client`)

The core stays minimal: only what every app needs to run and talk to its
backend.

`app`, `tyndWindow`, `tyndWebview` (current window and webview), `monitors`,
`log`, `security`, `os`, `path`, `workers`.

Every other native capability is a host-backed official plugin. These plugins
(`plugins/host/*` in the repo) declare the native host APIs they need, so the CLI compiles the matching
Cargo features into the app's host. `tynd add <name>` installs one and lists it
in `plugins`:

```ts
// tynd.config.ts
export default {
  plugins: ["@tynd/plugin-fs", "@tynd/plugin-updater"],
  backend: "backend/main.ts",
  frontendDir: "frontend",
}

// frontend
import { fs } from "@tynd/plugin-fs/client"
import { updater } from "@tynd/plugin-updater/client"
```

Host-backed official plugins:

| Capability | Package | Client import |
|---|---|---|
| Filesystem | `@tynd/plugin-fs` | `@tynd/plugin-fs/client` |
| Host HTTP | `@tynd/plugin-http` | `@tynd/plugin-http/client` |
| Updater | `@tynd/plugin-updater` | `@tynd/plugin-updater/client` |
| WebSocket client | `@tynd/plugin-websocket` | `@tynd/plugin-websocket/client` |
| Notifications | `@tynd/plugin-notification` | `@tynd/plugin-notification/client` |
| System stats | `@tynd/plugin-system-stats` | `@tynd/plugin-system-stats/client` |
| Window state | `@tynd/plugin-window-state` | `@tynd/plugin-window-state/client` |
| Process exec/spawn | `@tynd/plugin-process` | `@tynd/plugin-process/client` |
| Sidecars | `@tynd/plugin-sidecar` | `@tynd/plugin-sidecar/client` |
| Global shortcuts | `@tynd/plugin-shortcuts` | `@tynd/plugin-shortcuts/client` |
| Keyring | `@tynd/plugin-keyring` | `@tynd/plugin-keyring/client` |
| Autolaunch | `@tynd/plugin-autolaunch` | `@tynd/plugin-autolaunch/client` |
| Power monitor | `@tynd/plugin-power` | `@tynd/plugin-power/client` |
| Vault | `@tynd/plugin-vault` | `@tynd/plugin-vault/client` |
| Screenshot | `@tynd/plugin-screenshot` | `@tynd/plugin-screenshot/client` |
| Network interfaces | `@tynd/plugin-network` | `@tynd/plugin-network/client` |
| Deep links | `@tynd/plugin-deep-link` | `@tynd/plugin-deep-link/client` |
| Biometric auth | `@tynd/plugin-biometric` | `@tynd/plugin-biometric/client` |
| Dialogs | `@tynd/plugin-dialog` | `@tynd/plugin-dialog/client` |
| Clipboard | `@tynd/plugin-clipboard` | `@tynd/plugin-clipboard/client` |
| Key-value store | `@tynd/plugin-store` | `@tynd/plugin-store/client` |
| Tray | `@tynd/plugin-tray` | `@tynd/plugin-tray/client` |
| Menus | `@tynd/plugin-menu` | `@tynd/plugin-menu/client` |
| Secondary windows and child webviews | `@tynd/plugin-multiwindow` | `@tynd/plugin-multiwindow/client` |
| Shell open | `@tynd/plugin-shell` | `@tynd/plugin-shell/client` |
| Single instance | `@tynd/plugin-single-instance` | `@tynd/plugin-single-instance/client` |
| Theme | `@tynd/plugin-theme` | `@tynd/plugin-theme/client` |
| Key blocking | `@tynd/plugin-keys` | `@tynd/plugin-keys/client` |
| CLI arguments | `@tynd/plugin-cli` | `@tynd/plugin-cli/client` |
| Hashing and random bytes | `@tynd/plugin-compute` | `@tynd/plugin-compute/client` |

Other official plugins ship their own implementation instead of a host API,
such as `@tynd/plugin-sql` or `@tynd/plugin-terminal`. Community (third-party)
plugins are packages linked from
their own repositories through npm/git dependencies; their manifest omits
`tynd.kind`.

Other entry points: `@tynd/core/client/bridge` (`osCall`, `osOn`, `binFetch`, … — the host bridge plugin clients build on), `@tynd/core/plugin` (`definePlugin`, `createPluginClient`, no Bun or Node imports), `@tynd/core/testing` (`installTyndMock` for frontend unit tests) and `@tynd/core/env` (ambient types).

**Full reference:** [API docs](https://kvnpetit.github.io/tynd/docs/v1.0/api).

## Runtime

`@tynd/core` always runs on Bun. Every app runs on a native host compiled for
it with Cargo from `@tynd/host-src` (`tynd dev` uses the dev profile, `tynd build`
release), with the core APIs plus the ones its plugins declare. No prebuilt host
is published.

## Related packages

- [`@tynd/cli`](https://www.npmjs.com/package/@tynd/cli) — `tynd create / dev / build / start`
- [`@tynd/host-src`](https://www.npmjs.com/package/@tynd/host-src) — Rust host sources compiled into each app's host

## License

Apache-2.0
