# @tynd/cli

The CLI for [Tynd](https://github.com/kvnpetit/tynd) — scaffold, dev, build, and ship native desktop apps in TypeScript.

## Prerequisites

**[Bun](https://bun.sh) is required.** The CLI binary has a `#!/usr/bin/env bun` shebang and uses Bun-native APIs (`Bun.build`, `Bun.file`, `Bun.spawn`, `Bun.hash`) — it will not run under Node.js.

```bash
# macOS / Linux / WSL
curl -fsSL https://bun.sh/install | bash

# Windows (PowerShell)
powershell -c "irm bun.sh/install.ps1 | iex"
```

End users of apps you ship with `tynd build` do **not** need Bun — it's packed into the final binary (full mode) or replaced by an embedded JS engine (lite mode).

## Quick start

```bash
bunx @tynd/cli create my-app
cd my-app
tynd dev
```

## Commands

```bash
tynd create [name]           # Scaffold a new project (interactive if no args)
  --framework <fw>           # react | vue | svelte | solid | preact | lit | angular
  --runtime   <r>            # lite (recommended) | full

tynd dev                     # Dev mode: HMR frontend + hot-reload backend
tynd start                   # Run from cached bundles, no rebuild, no watcher
tynd build                   # Single-file distributable binary
  --bundle [targets]         # + native installers: app,dmg,deb,rpm,appimage,nsis,msi
  --outfile <path>           # Custom output path (default: release/<name>[.exe])

tynd init                    # Add Tynd to an existing project
tynd clean                   # Remove build artifacts (.tynd/cache, release/)
tynd validate                # Check config + binary availability
tynd info [--json]           # Environment diagnostics (Bun, Rust, WebView2, paths)
tynd upgrade                 # Bump @tynd/* deps to latest

# Global flags
tynd --verbose <cmd>         # Debug-level logs (also sets TYND_LOG=debug in host)
tynd --quiet <cmd>           # Errors only
```

## Scaffolded layout

```
my-app/
├── tynd.config.ts        ← runtime + paths + bundle metadata
├── package.json
├── backend/
│   └── main.ts           ← backend entry — app.start() here
└── src/
    └── main.tsx          ← frontend source (React/Vue/…)
```

## Supported frameworks

React, Vue, Svelte, Solid, Preact, Lit, Angular. Vite-backed except Angular (uses Angular CLI). Any SPA works via `tynd init`.

**Blocked (SSR):** Next.js, Nuxt, SvelteKit, Remix, Gatsby, SolidStart, Angular Universal, Qwik City, Astro, TanStack Start, Vike. Use the pure-SPA variant.

See [FRAMEWORKS.md](https://github.com/kvnpetit/tynd/blob/main/FRAMEWORKS.md).

## Typed config

```ts
import type { TyndConfig } from "@tynd/cli"

export default {
  runtime: "lite",
  backend: "backend/main.ts",
  frontendDir: "dist",
  window: { title: "My App", width: 1200, height: 800 },
  bundle: { identifier: "com.example.myapp" },  // required for --bundle
} satisfies TyndConfig
```

## Installers (`tynd build --bundle`)

| Host OS | Formats |
|---|---|
| macOS | `.app`, `.dmg` |
| Linux | `.deb`, `.rpm` (needs `rpmbuild`), `.AppImage` |
| Windows | NSIS `.exe` setup, `.msi` (via WiX v3) |

Build tools (NSIS, WiX, appimagetool) auto-download to `.tynd/cache/tools/` on first use. Cross-compilation is **not** supported — run on each target host (see the `build-host.yml` workflow in the repo).

**Code signing** is post-build — see [SIGNING.md](https://github.com/kvnpetit/tynd/blob/main/SIGNING.md).

## Related packages

- [`@tynd/core`](https://www.npmjs.com/package/@tynd/core) — backend + frontend API surface
- [`@tynd/host`](https://www.npmjs.com/package/@tynd/host) — prebuilt native binaries (auto-installed)

## License

Apache-2.0
