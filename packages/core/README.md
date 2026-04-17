# @tynd/core

The backend API and frontend client for [Tynd](https://github.com/kvnpetit/tynd) — desktop apps in TypeScript.

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

All Rust-backed — same surface in both the `full` and `lite` runtimes:

`dialog`, `tyndWindow`, `clipboard`, `shell`, `notification`, `tray`, `process`, `fs`, `store`, `os`, `path`, `http`, `websocket`, `sql`, `sidecar`, `terminal`, `compute`, `workers`, `parallel`, `singleInstance`, `crashReporter`.

**Full reference:** [API.md](https://github.com/kvnpetit/tynd/blob/main/API.md).

## Runtime modes

`@tynd/core` runs on either of Tynd's two backend hosts:

| | `lite` | `full` |
|---|---|---|
| JS engine | embedded interpreter (in the Rust binary) | Bun subprocess with JIT |
| Binary size | ~6.5 MB + assets | ~44 MB (Bun packed) |
| Best for | most desktop apps | CPU-bound JS / npm native bindings |

The CLI's bundler swaps in the right module at build time via a `define`-based dead-code-elimination trick — only one runtime ships in any given binary.

See [RUNTIMES.md](https://github.com/kvnpetit/tynd/blob/main/RUNTIMES.md).

## Related packages

- [`@tynd/cli`](https://www.npmjs.com/package/@tynd/cli) — `tynd create / dev / build / start`
- [`@tynd/host`](https://www.npmjs.com/package/@tynd/host) — prebuilt Rust binaries (downloaded by postinstall)

## License

Apache-2.0
