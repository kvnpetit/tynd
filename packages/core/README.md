# @tynd/core

The backend API and frontend client for [Tynd](https://github.com/kvnpetit/tynd) — desktop apps in TypeScript.

## Prerequisites

**[Bun](https://bun.sh) is required** during development. Tynd's backend runs on Bun (full mode) or an embedded JS engine via `@tynd/cli`'s bundler (lite mode); Node.js is not a supported substitute. Install Bun once:

```bash
# macOS / Linux / WSL
curl -fsSL https://bun.sh/install | bash

# Windows (PowerShell)
powershell -c "irm bun.sh/install.ps1 | iex"
```

End users of built apps need nothing — the runtime ships inside the binary.

## Install

```bash
bun add @tynd/core @tynd/host
```

## Two entrypoints

```ts
// backend/main.ts  (Bun process)
import { app, createEmitter } from "@tynd/core"

// src/App.tsx  (WebView)
import { createBackend, fs, dialog, sql /* …and more */ } from "@tynd/core/client"
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
import { createBackend, fs, dialog } from "@tynd/core/client"
import type * as backend from "../backend/main"

const api = createBackend<typeof backend>()
const msg = await api.greet("Alice")   // string ✅ — types flow from backend
api.on("ready", ({ message }) => console.log(message))

// Direct OS APIs — bypass the backend
const file = await dialog.openFile()
const bytes = await fs.readBinary(file!)
```

## OS APIs (`@tynd/core/client`)

Native integrations — identical surface in both `full` and `lite` modes:

`app`, `dialog`, `tyndWindow`, `monitors`, `menu`, `clipboard`, `shell`, `notification`, `tray`, `process`, `fs` (+ watcher, handles, symlinks), `shortcuts`, `keyring`, `autolaunch`, `store`, `updater`, `log`, `power`, `security`, `os`, `path`, `http`, `websocket`, `sql`, `sidecar`, `terminal`, `compute`, `workers`, `singleInstance`.

Plus Web-standard re-exports (`fetch`, `WebSocket`, `crypto`, `URL`, `Blob`, `AbortController`, `TextEncoder`, …) so `import * as tynd from "@tynd/core/client"` gives you the whole namespace at once.

**Full reference:** [API.md](https://github.com/kvnpetit/tynd/blob/main/API.md).

## Runtime modes

`@tynd/core` runs on either of Tynd's two native hosts:

| | `lite` | `full` |
|---|---|---|
| JS runtime | embedded interpreter, in-process | Bun, packed into the binary |
| Binary size | ~6.5 MB + assets | ~44 MB (Bun compressed) |
| Best for | most desktop apps | CPU-bound JS / npm native bindings |

The CLI's bundler swaps in the right module at build time via a `define`-based dead-code-elimination trick — only one runtime ships in any given binary.

See [RUNTIMES.md](https://github.com/kvnpetit/tynd/blob/main/RUNTIMES.md).

## Related packages

- [`@tynd/cli`](https://www.npmjs.com/package/@tynd/cli) — `tynd create / dev / build / start`
- [`@tynd/host`](https://www.npmjs.com/package/@tynd/host) — prebuilt native binaries (downloaded by postinstall)

## License

Apache-2.0
