# Migration guide

This guide covers upgrading between Tynd minor versions. Each section
lists the user-visible changes that might require a code edit on your
side, with the commit ref when deeper context helps.

The full list of every landed change lives in [CHANGELOG.md](./CHANGELOG.md) —
this file only highlights what *you* might have to touch.

---

## Upgrading from v0.3 → v1.0

v1.0 ships **one breaking change** — most of the OS surface moved out
of `@tynd/core` into individual `@tynd/plugin-*` packages. Preset
runtimes (`full` and `lite`) still bundle every first-party plugin, so
a preset app keeps every capability it had in v0.3; only the import
paths change.

Custom runtimes (`full-custom` / `lite-custom`) pick plugins à la carte
through the `plugins` array in `tynd.config.ts`. Dropping in a
third-party plugin now works exactly like a first-party one.

### 1. Update the runtime packages

```bash
bun add -d @tynd/cli@^1
bun add    @tynd/core@^1 @tynd/host@^1
bun install
```

Preset apps don't need to list the plugins explicitly — they're
transitively installed through `@tynd/core` and registered by the CLI
at build time. Custom apps add them to `tynd.config.ts::plugins`:

```ts
// tynd.config.ts
export default {
  runtime: "lite-custom",
  apis: ["fs", "http", "dialog"],
  plugins: ["@tynd/plugin-sql", "@tynd/plugin-keyring"],
  backend: "backend/main.ts",
  frontendDir: "dist",
}
```

### 2. Rewrite imports for the extracted APIs

Frontend imports move to the matching plugin package:

| Old import (v0.3)                                     | New import (v1.0)                                           |
|-------------------------------------------------------|-------------------------------------------------------------|
| `import { sql } from "@tynd/core/client"`             | `import { sql } from "@tynd/plugin-sql/client"`             |
| `import { terminal } from "@tynd/core/client"`        | `import { terminal } from "@tynd/plugin-terminal/client"`   |
| `import { updater } from "@tynd/core/client"`         | `import { updater } from "@tynd/plugin-updater/client"`     |
| `import { keyring } from "@tynd/core/client"`         | `import { keyring } from "@tynd/plugin-keyring/client"`     |
| `import { autolaunch } from "@tynd/core/client"`      | `import { autolaunch } from "@tynd/plugin-autolaunch/client"` |
| `import { shortcuts } from "@tynd/core/client"`       | `import { shortcuts } from "@tynd/plugin-shortcuts/client"` |
| `import { log } from "@tynd/core/client"`             | `import { log } from "@tynd/plugin-log/client"`             |
| `import { power } from "@tynd/core/client"`           | `import { power } from "@tynd/plugin-power/client"`         |

A mixed import stays mixed — split the old one into a core line plus
one plugin line per extracted symbol:

```ts
// Before
import { shortcuts, tyndWindow } from "@tynd/core/client"

// After
import { tyndWindow } from "@tynd/core/client"
import { shortcuts } from "@tynd/plugin-shortcuts/client"
```

Every other `@tynd/core/client` import (`fs`, `http`, `websocket`,
`dialog`, `tray`, `menu`, `notification`, `clipboard`, `shell`,
`process`, `sidecar`, `singleInstance`, `store`, `workers`, `security`,
`app`, `os`, `path`, `tyndWindow`, `compute`) stays on `@tynd/core/client`.

### 3. Backend imports are unchanged

`@tynd/core` still exports `app`, `createEmitter`, `onFrontendEmit`,
`evalInFrontend`, `definePlugin`, `registerPlugin`,
`clearFrontendListeners`, and every type. Your `backend/main.ts` is
source-compatible with v0.3.

### 4. Third-party plugins

The new `@tynd/plugin-sdk` package + `tynd_plugin_sdk` Rust crate let
any external package ship a JS-only or native plugin with the same
shape as the first-party ones. Authors expose a single
`definePlugin({...})` (JS) or `register_plugin!(...)` (Rust) and
consumers register them through `tynd.config.ts::plugins`. See
[PLUGINS.md](./PLUGINS.md) for the authoring walkthrough.

### 5. Plugin capability gate (optional)

Native plugins can be scoped to a subset of core OS APIs from the app
config. Calls through `cx.os_call(api, method, args)` that target an
API outside the list fail with a `"plugin capability: not permitted"`
error before reaching the dispatcher.

```ts
// tynd.config.ts
export default {
  runtime: "lite-custom",
  apis: ["fs", "http", "store"],
  plugins: [
    "@tynd/plugin-clipboard-history",                  // unrestricted (default)
    { pkg: "@acme/plugin-telemetry", apis: ["http"] }, // http-only
    { pkg: "@acme/plugin-vault",     apis: ["fs"]   }, // fs-only
  ],
  backend: "backend/main.ts",
  frontendDir: "dist",
}
```

The gate only covers `cx.os_call`. A plugin still has direct access to
`std::fs`, `std::net`, etc. — the field expresses *intent*, not
sandboxing. Pair with `security.configure({ fs, http })` at the app
level if you need hard enforcement.

JS-only plugins ignore `apis`: they run in the user's backend process
with the same access as user code.

### 6. What did *not* change

- IPC wire format — `{type:"call"|"cancel"|"ack"|"yield"|"return"}`.
- `app.start`, `createEmitter`, `createBackend`, `createPluginClient`.
- TYNDPKG pack format / bundle targets / signing config.
- Minimum Bun version (`>= 1.3.0`).
- The public shape of any extracted API — only the import path moves.

---

## Upgrading from v0.2 → v0.3

v0.3 is **additive and source-compatible with v0.2**. Every v0.2 app
keeps building and running unchanged. The new surface is opt-in; the
items below explain what you can pick up without rewriting anything.

### 1. Update the CLI + runtime packages

```bash
bun add -d @tynd/cli@^0.3
bun add    @tynd/core@^0.3 @tynd/host@^0.3
bun install
```

`@tynd/host`'s postinstall downloads the matching native binaries the
first time. If your `package.json` was scaffolded before v0.3, add
`"@tynd/host"` to `trustedDependencies` so Bun runs that hook without
an explicit `bun pm trust`.

### 2. New backend → frontend eval

`evalInFrontend(script, label?)` runs raw JS inside any WebView — handy
for DevTools-style helpers or forcing a re-init without a reload. Omit
`label` to broadcast to every open window.

```ts
import { evalInFrontend } from "@tynd/core"
evalInFrontend(`document.body.classList.add("debug")`)
evalInFrontend(`window.location.reload()`, "settings")
```

### 3. Frontend → backend events

`createFrontendEmitter()` (frontend) + `onFrontendEmit()` (backend) now
provide a typed bus going the opposite direction of `createEmitter`.
Register backend handlers once; they survive frontend navigations. On
`app.onClose` the listener map is cleared automatically — you can also
call `clearFrontendListeners()` manually in tests.

### 4. Window API expansion

All new; no v0.2 APIs renamed or removed.

- `tyndWindow.positionAt(preset)` — 13 presets (`"center"`,
  `"top-right"`, `"cursor"`, …).
- `tyndWindow.setProgressBar(0..1)` — taskbar progress on every OS.
- Mica / Acrylic / Tabbed backdrop (Windows 11):
  `tyndWindow.setSystemBackdrop("mica")`.
- macOS polish: `titlebar: "hidden-overlay"`, traffic-light inset,
  `setShadow`, `setVisibleOnAllWorkspaces`.
- `findInPage` / `stopFindInPage` across all OS.
- `print` / `navigate` / `loadHtml` / `getUrl` / `setBadge` /
  `setContentProtection` / `setSkipTaskbar`.

### 5. Security (opt-in capability ACL)

Leave `security` unset and v0.2 behavior is preserved (every API is
allowed). If you want finer control:

```ts
app.start({
  security: {
    fs:   { allow: ["$APP_DATA/**"], deny: ["$HOME/.ssh/**"] },
    http: { allow: ["https://api.example.com/**"] },
  },
})
```

Deny overrides allow; matching uses glob (`*` / `**`). Violations
throw at the API boundary.

### 6. Notifications: schedule + actions

```ts
await notification.schedule({ title: "Remind me", delayMs: 60_000 })
notification.onAction((id) => console.log("clicked", id))
```

### 7. Tray runtime mutation + mouse events

Previously the tray config was immutable after `app.start`. Now:

```ts
tray.setIcon("/path/to/new.png")
tray.setTooltip("Updated")
tray.setMenu(newMenuItems)
tray.onEnter(() => …)
tray.onLeave(() => …)
```

### 8. Updater: periodic check + downgrade flag

The manifest format is unchanged. New knobs:

```ts
updater.startPeriodicCheck({
  endpoint,
  currentVersion,
  intervalMs: 3600_000,
  proxy: "http://proxy:8080",
  headers: { Authorization: "Bearer …" },
  allowDowngrade: false,
})
```

### 9. Log / Power / Security APIs

Three brand-new top-level APIs — import them only if you use them.
The log API writes JSON-lines with size rotation; `captureConsole()`
bridges `console.*` into the file sink without silencing DevTools.

### 10. What did *not* change

- IPC wire format — `{type:"call"|"cancel"|"ack"|"yield"|"return"}`.
- `app.start`, `createEmitter`, `createBackend`, `createPluginClient`.
- TYNDPKG pack format.
- `tynd build --bundle` targets (`.app`, `.dmg`, `.deb`, `.rpm`,
  `.AppImage`, NSIS `.exe`, `.msi`).
- Signing config shape (`bundle.sign.windows` / `bundle.sign.macos`).
- Minimum Bun version (`>= 1.3.0`).

---

## Reporting issues with migration

If an upgrade breaks something that isn't listed here, open an issue
with the failing code and the delta — we'll either patch the guide or
the library.
