# Tynd

Tynd is a TypeScript-first desktop app framework built on native OS webviews,
a Rust host and native Rust developer tooling. Bun runs application code.

Documentation: <https://tynd-apps.github.io/tynd/>

## Quick Start

Tynd needs [Bun](https://bun.sh) and [Rust](https://rustup.rs) 1.95 or newer
(plus the platform build tools) during development; you write no Rust, but every
app compiles its own native host. End users of built apps need nothing.

```bash
bunx @tynd/cli create my-app
cd my-app
tynd dev        # run with hot reload
tynd build      # produce the release binary
```

See [Installation](https://tynd-apps.github.io/tynd/docs/v1.0/getting-started/installation)
and [First app](https://tynd-apps.github.io/tynd/docs/v1.0/getting-started/first-app).

## At A Glance

- Application backend, frontend, IPC surface, and config stay TypeScript-first.
- `tynd`, `tynd-mcp`, `tynd-bundler`, the desktop host, and the runtime are
  native Rust executables.
- `@tynd/cli` and `@tynd/mcp` expose tiny npm launchers that resolve the
  matching platform binary from `@tynd/tooling`.
- macOS, Linux, and Windows packages are published for x64 and ARM64.
- Native WebView2 / WKWebView / WebKitGTK window, with no bundled browser.
  macOS apps require macOS 14 or later.
- A minimal host core: window/WebView lifecycle, `tynd://`, IPC, RPC and
  events, app lifecycle, the debug protocol, ACL, and logging. Every other host
  API (dialog, clipboard, menu, tray, multiwindow, fs, http, updater, ...) is a
  official plugin `@tynd/plugin-<name>`; `tynd add <name>` installs it and lists
  it in `plugins` in `tynd.config.ts`, and the host is compiled with just those
  APIs. Frontend code imports it from `@tynd/plugin-<name>/client`.
- Bun provides the runtime for user backend code, frontend commands, package
  scripts, Node compatibility, native addons, and JIT execution.
- The host is always compiled per app with cargo from the Rust sources in the
  `@tynd/host-src` dev dependency (`tynd dev` uses the dev profile, `tynd build`
  release); no prebuilt host is published.

## CLI

```bash
tynd create <name>
  --framework react|vue|svelte|solid|preact|lit|angular

tynd add <capability-or-plugin>
tynd dev
tynd start
tynd test
tynd build
tynd build --bundle <targets>
tynd init
tynd clean
tynd validate
tynd info
tynd doctor
tynd upgrade
tynd updater publish <version>
tynd updater release <version>
```

Other commands: `remove`, `config show`, `explain`, `size`, `report`,
`profile`, `completions`, the plugin group `plugin new|check|dev|list|types`
and the updater group `updater keygen|sign|publish|release|server`. See the
[CLI reference](https://tynd-apps.github.io/tynd/docs/v1.0/cli) or
`tynd <command> --help`.

## Config

```ts
import type { TyndConfig } from "@tynd/cli";

export default {
  backend: "backend/main.ts",
  frontendDir: "dist",
  icon: "public/favicon.svg",
  plugins: ["@tynd/plugin-fs"],
  bundle: {
    identifier: "com.example.myapp",
  },
} satisfies TyndConfig;
```

| Field | Default | Description |
| ----- | ------- | ----------- |
| `plugins` | `[]` | Plugins to bundle. Object entries accept command-level native `permissions`, API-wide `apis`, and additive Cargo `features` |
| `apis` | `[]` | Extra native APIs compiled into the host |
| `backend` | `"backend/main.ts"` | Backend entry file |
| `frontendDir` | `"frontend"` | Built frontend output directory |
| `devUrl` | auto | Dev server URL override |
| `devCommand` | auto | Dev server start command override |
| `icon` | - | Explicit SVG / PNG / ICO app icon path; native fallback used when omitted |
| `binaryArgs` | - | Extra args passed after `--` to the backend (`process.argv`) |

Other keys include `extends`, `devCommandArgs`, `protocols`, `sidecars`,
`bundle`, `updater` and `observability`. The
[config reference](https://tynd-apps.github.io/tynd/docs/v1.0/cli/config) lists
every key and its default.

## Native Tooling Distribution

`@tynd/tooling` selects one platform package through npm
`optionalDependencies`. Each platform package contains all three developer
executables:

```text
@tynd/cli ── launcher ──┐
                        ├─ @tynd/tooling-<os>-<arch>/bin/
@tynd/mcp ── launcher ──┘    ├─ tynd
                             ├─ tynd-mcp
                             └─ tynd-bundler
```

The launcher only forwards arguments and exit status. CLI, MCP protocol
handling, project orchestration, and native packaging execute in Rust. Bun is
kept for the TypeScript application backend and project-defined commands.

## Building From Source

Build the native developer tools:

```bash
cargo build --release -p tynd-cli --bins
cargo build --release -p tynd-bundler
```

Build the application host:

```bash
cargo build --release -p tynd-runtime
```

Linux host development dependencies:

```bash
sudo apt install libgtk-3-dev libwebkit2gtk-4.1-dev \
  libjavascriptcoregtk-4.1-dev libsoup-3.0-dev librsvg2-dev \
  libxdo-dev libxss-dev libpipewire-0.3-dev libgbm-dev \
  libdrm-dev libclang-dev rpm patchelf
```

## Repo Layout

```text
apps/
  docs/         documentation site
crates/
  capabilities/ pure capability and ACL contracts
  debug-protocol/ native debugging protocol and taps
  protocol/     shared protocol types
  pack-format/  binary pack/update format helpers
  update-core/  updater manifest and patch logic
  security-policy/ pure security policy engine
  host/         native host library
  host-macros/  host dispatch macros
  runtime/      tynd-runtime application host
  cli/          native tynd and tynd-mcp executables
  bundler/      tynd-bundler packaging executable
  bundle-contract/ shared CLI/bundler contract types
  plugin-sdk/   native plugin SDK (tynd-plugin-sdk)
  plugin-macros/ plugin proc macros
npm/
  tooling/      @tynd/tooling native developer-tool resolver
  tooling-*/    platform packages for tynd, tynd-mcp, and tynd-bundler
packages/
  host-src/     @tynd/host-src source mirror compiled into each app host
  core/         @tynd/core TypeScript application API
  cli/          @tynd/cli launcher and typed config surface
  mcp/          @tynd/mcp launcher
  plugin-sdk/   plugin SDK
  test/         @tynd/test end-to-end test harness
plugins/
  host/         host API facades
  standalone/   official JavaScript and native plugins
```

## License

Apache-2.0. See [LICENSE](./LICENSE) and [NOTICE](./NOTICE).
