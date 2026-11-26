# @tynd/cli

[![License](https://img.shields.io/badge/license-Apache%202.0-blue.svg)](LICENSE)
[![CI](https://github.com/kvnpetit/tynd/actions/workflows/ci.yml/badge.svg)](https://github.com/kvnpetit/tynd/actions/workflows/ci.yml)
[![npm](https://img.shields.io/npm/v/@tynd/cli.svg)](https://www.npmjs.com/package/@tynd/cli)

The native Rust CLI for [Tynd](https://github.com/kvnpetit/tynd): scaffold,
develop, build, and ship desktop apps whose application code stays TypeScript.

## Runtime model

The published npm package contains:

- a small launcher for the native `tynd` executable;
- the dependency-free public `TyndConfig` TypeScript surface;
- the config JSON Schema (`schemas/tynd.config.json`).

The launcher resolves the current platform through `@tynd/tooling` and forwards
arguments, stdio, signals, and exit status to Rust. There is no install script
and no binary download: npm optional dependencies supply the platform package.

[Bun](https://bun.sh) and [Rust](https://rustup.rs) 1.95 or newer remain
required by Tynd application projects: Cargo compiles each app's host. Bun runs
user backend TypeScript, frontend commands, package scripts, and bundled
application code. CLI orchestration, project inspection, and native packaging
execute in Rust.

End users of packaged applications do not install Bun separately; Tynd embeds
the matching runtime in the application.

## Install

```bash
bun add --global @tynd/cli

# One-shot
bunx @tynd/cli
```

## Quick start

```bash
bunx @tynd/cli create my-app
cd my-app
tynd dev
```

## Native package layout

```text
@tynd/cli/bin/index.js
  └─ @tynd/tooling
       └─ @tynd/tooling-<os>-<arch>/bin/
            ├─ tynd[.exe]
            ├─ tynd-mcp[.exe]
            └─ tynd-bundler[.exe]
```

Supported targets are macOS, Linux, and Windows on x64 and ARM64.

## Commands

The Rust command surface is:

```text
Project:     create, init, add, remove, dev, start, test, build, clean, upgrade
Inspect:     validate, doctor, info, size, explain, config show, report, profile
Plugins:     plugin new, plugin check, plugin dev, plugin list, plugin types
Updater:     updater keygen, updater sign, updater publish, updater release,
             updater server
Shell:       completions
```

Common examples:

```bash
tynd create my-app --framework react
tynd add fs
tynd dev
tynd start
tynd build --bundle all
tynd validate --ci
tynd info --json
tynd doctor --fix
tynd config show --json
tynd plugin new example --native
tynd updater publish 1.2.3 --target manifest-only
tynd completions powershell
```

`tynd completions` generates a completion script at runtime; static completion
files are not included in the npm package. Run `tynd <command> --help` for the
authoritative arguments of a command.

Global flags include `--verbose`, `--quiet`, `--no-color`, `--env`,
`--log-format`, and `--log-file`.

## Typed config

JSON, YAML, and TOML users can reference the published JSON Schema. TypeScript
users keep the public `@tynd/cli` type:

```ts
import type { TyndConfig } from "@tynd/cli";

export default {
  backend: "backend/main.ts",
  frontendDir: "dist",
  bundle: {
    identifier: "com.example.myapp",
  },
} satisfies TyndConfig;
```

The config type remains TypeScript even though command execution is Rust.

## Application runtime and native APIs

Tynd runs backend TypeScript on Bun. Every app runs on a Rust host that
`tynd dev` (cargo dev profile) and `tynd build` (release) compile from
`@tynd/host-src`, so Rust 1.95 or newer is required. `tynd start` runs the
binary from a previous `tynd build`.

The host core covers the window, WebView, `tynd://`, IPC, lifecycle, logging,
and ACL. Every other host API (clipboard, dialog, fs, menu, tray, ...) is an
official plugin: `tynd add <name>` installs `@tynd/plugin-<name>` and lists it in
`plugins`. `apis` adds extra host APIs, and native plugin permission lists are
compiled into the host as well.

## Installers

`tynd build --bundle <targets>` delegates packaging to the Rust `tynd-bundler` executable
shipped beside `tynd`.

| Host OS | Formats |
| --- | --- |
| macOS | `.app`, `.dmg` |
| Linux | `.deb`, `.rpm`, `.AppImage` |
| Windows | NSIS `.exe`, `.msi` |

Some formats require their platform toolchain, such as WiX, NSIS, `rpmbuild`,
or AppImage utilities. The bundler reports a missing format-specific
prerequisite only when that format is requested.

## Build from source

From the repository root:

```bash
cargo build --release -p tynd-cli --bins
cargo build --release -p tynd-bundler
```

## Related packages

- [`@tynd/tooling`](https://www.npmjs.com/package/@tynd/tooling) — native
  `tynd`, `tynd-mcp`, and `tynd-bundler` distribution
- [`@tynd/core`](https://www.npmjs.com/package/@tynd/core) — application API
- [`@tynd/host-src`](https://www.npmjs.com/package/@tynd/host-src) — Rust
  host sources compiled into each app's host
- [`@tynd/mcp`](https://www.npmjs.com/package/@tynd/mcp) — native MCP server

## License

Apache-2.0
