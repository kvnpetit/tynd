# Tynd plugins

[![License](https://img.shields.io/badge/license-Apache%202.0-blue.svg)](LICENSE)
[![CI](https://github.com/kvnpetit/tynd/actions/workflows/ci.yml/badge.svg)](https://github.com/kvnpetit/tynd/actions/workflows/ci.yml)

Standalone official Tynd plugins workspace — native and pure-JS plugins for the [Tynd](https://github.com/kvnpetit/tynd) framework.

## Plugins

| Package | Description |
|---|---|
| [`@tynd/plugin-audio`](./audio) | Audio capture / playback — `cpal` + WAV via `hound` |
| [`@tynd/plugin-oauth`](./oauth) | Loopback OAuth callback listener with PKCE-style state validation |
| [`@tynd/plugin-sql`](./sql) | Bundled SQLite via `bun:sqlite` |
| [`@tynd/plugin-terminal`](./terminal) | Embedded PTY — spawn shells, stream output, resize, real `kill()` |

> **Host API split:** the host core only compiles window/webview, IPC,
> lifecycle, `acl` and `log`. Other host APIs (autolaunch, biometric,
> deep-link, power, shortcuts, updater, vault, network, ...) are
> host-backed official plugins under [`plugins/host`](../host). The plugins here
> cover terminal/PTY, OAuth callbacks, SQL and audio.
> For i18n or QR codes, use npm libraries
> directly (`i18next` / `lingui` / `formatjs`; `qrcode`).

## Plugin types

**Bun backend** — optional plugins bundled into the app backend:
`oauth`, `sql`

**Native (Rust)** — ship a Rust crate compiled into the app's host:
`audio` and `terminal`

Native packages declare the Cargo features required for their advertised
baseline API. The CLI enables those features automatically. An app only lists
extra features when it wants an optional accelerator or codec:

```ts
plugins: [
  { pkg: "@tynd/plugin-audio", features: ["capture"] },
]
```

The feature-less Cargo build remains a lightweight stub for direct Rust
consumers and workspace tooling; it is not the build produced by a normal Tynd
plugin installation.

## Status

Stable published plugins ship at `1.0.x`. Stable native-backed plugins
(`audio` and `terminal`) carry the same SemVer guarantee as the rest, but
their platform backends evolve independently — pin exact versions in
production until your native deps stabilise.

## Development

Run the shared workspace check from the repository root:

```bash
bun run check
```

## Writing a plugin

See [@tynd/plugin-sdk](https://github.com/kvnpetit/tynd/tree/main/packages/plugin-sdk) for the JS and Rust API surface.

```ts
// backend.ts
import { definePlugin } from "@tynd/plugin-sdk"

export default definePlugin({
  name: "my-plugin",
  methods: {
    greet(name: string) { return `Hello, ${name}!` },
  },
})
```

```ts
// client.ts
import { createPluginClient } from "@tynd/plugin-sdk"
import type Plugin from "./backend"

export const myPlugin = createPluginClient<typeof Plugin>("my-plugin")
```

A plugin with a Bun backend sets `"backend": true` under `tynd` in
`package.json` and exports `./backend` (`backend.ts`) next to `./client`.

Declare `tynd.native` in `package.json` for plugins that ship native Rust code:

```json
{
  "tynd": {
    "native": {
      "path": "./native",
      "crateName": "plugin_my_plugin"
    }
  }
}
```

## Related repos

- [`@tynd/cli`](https://github.com/kvnpetit/tynd/tree/main/packages/cli) — scaffold, dev, build, ship
- [`@tynd/core`](https://github.com/kvnpetit/tynd) (in `kvnpetit/tynd`) — core OS APIs and runtime
- [`@tynd/plugin-sdk`](https://github.com/kvnpetit/tynd/tree/main/packages/plugin-sdk) — plugin authoring SDK
- [`@tynd/mcp`](https://github.com/kvnpetit/tynd/tree/main/packages/mcp) — Model Context Protocol server
- [Documentation site](https://github.com/kvnpetit/tynd/tree/main/apps/docs)

## License

Apache-2.0
