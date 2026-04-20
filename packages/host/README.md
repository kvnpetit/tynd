# @tynd/host

Prebuilt native host binaries for [Tynd](https://github.com/kvnpetit/tynd) — `tynd-full.exe` and `tynd-lite.exe` (plus the platform variants).

## Prerequisites

**[Bun](https://bun.sh) is required** — the `postinstall` script is `bun run install.ts` and the parent `@tynd/cli` toolchain is Bun-only. Node.js will not run it.

```bash
# macOS / Linux / WSL
curl -fsSL https://bun.sh/install | bash

# Windows (PowerShell)
powershell -c "irm bun.sh/install.ps1 | iex"
```

## Install

```bash
bun add @tynd/host
```

**You almost certainly don't install this directly.** `@tynd/cli`'s `tynd create` adds it as a peer automatically, and `@tynd/core` depends on it at runtime via the CLI's build pipeline.

## What it ships

On `bun install`, a `postinstall` script downloads the matching native binary from the GitHub Release that corresponds to the package version:

| Platform | File |
|---|---|
| Windows x64 / arm64 | `tynd-{full,lite}.exe` |
| macOS x64 / arm64 | `tynd-{full,lite}` |
| Linux x64 / arm64 | `tynd-{full,lite}` |

Binaries land in `node_modules/@tynd/host/bin/<plat>-<arch>/`. The CLI's `findBinary` (in `@tynd/cli`) picks them up automatically at `tynd dev` / `tynd build` time.

**Skipped inside the Tynd monorepo** — if the path contains the workspace layout, postinstall exits early and lets `cargo build` produce the binaries from source instead.

## What's inside the binaries

- `tynd-full` — packs Bun alongside the native host and runs your TypeScript on Bun's JIT. Heavier binary, full npm support (including native bindings).
- `tynd-lite` — embeds a lightweight JS engine inside the host binary. ~6.5 MB, no external runtime, pure-JS npm packages only.

Both share the same native host: the WebView event loop, the `tynd://` + `tynd-bin://` custom IPC schemes, and all OS APIs.

See [RUNTIMES.md](https://github.com/kvnpetit/tynd/blob/main/RUNTIMES.md).

## Building from source

If you don't want the prebuilt binaries:

```bash
cargo build --release -p tynd-full
cargo build --release -p tynd-lite
```

Linux dev deps:

```bash
sudo apt install libgtk-3-dev libwebkit2gtk-4.1-dev \
  libjavascriptcoregtk-4.1-dev libsoup-3.0-dev \
  libxdo-dev
```

## Release integrity

Every release is built by `.github/workflows/build-host.yml` with matching `actions/attest-build-provenance` attestations. The workflow runs only on tagged commits.

## License

Apache-2.0
