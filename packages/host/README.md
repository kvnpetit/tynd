# @tynd/host

Prebuilt native host binaries for [Tynd](https://github.com/tynd-apps/tynd) — `tynd-full` and `tynd-lite`, distributed via npm `optionalDependencies` (one binary per platform/arch sub-package).

## Install

```bash
bun add @tynd/host
```

**You almost certainly don't install this directly.** `@tynd/cli`'s `tynd create` adds it as a peer automatically, and `@tynd/core` depends on it at runtime via the CLI's build pipeline.

## How it works

`@tynd/host` is a wrapper package. It declares one platform-specific sub-package per supported target as an `optionalDependency`:

| Sub-package | OS | Arch |
|---|---|---|
| `@tynd/host-darwin-x64` | macOS | Intel |
| `@tynd/host-darwin-arm64` | macOS | Apple Silicon |
| `@tynd/host-linux-x64` | Linux | x64 |
| `@tynd/host-linux-arm64` | Linux | ARM64 |
| `@tynd/host-win32-x64` | Windows | x64 |
| `@tynd/host-win32-arm64` | Windows | ARM64 |

Each sub-package declares its `os` + `cpu` constraints in its `package.json`. npm/bun installs **only the matching one** for the host platform (the others are silently skipped via standard `optionalDependencies` semantics — same pattern as esbuild, swc, rolldown, biome).

No postinstall script. No GitHub Releases fetch. No `--ignore-scripts` issues. The CLI's `findBinary` resolves `node_modules/@tynd/host-<plat>-<arch>/bin/tynd-<runtime>` directly.

## Skipped inside the Tynd monorepo

If `findBinary` walks up and finds a `target/release/` or `target/debug/` directory in a parent (workspace dev), it uses that instead — `cargo build -p tynd-{full,lite}` is the dev loop.

## What's inside the binaries

- `tynd-full` — packs Bun alongside the native host and runs your TypeScript on Bun's JIT. Heavier binary, full npm support (including native bindings).
- `tynd-lite` — embeds a lightweight JS engine (rquickjs) inside the host binary. ~6.5 MB, no external runtime, pure-JS npm packages only.

Both share the same native host: the WebView event loop, the `tynd://` + `tynd-bin://` custom IPC schemes, and all OS APIs.

See the [runtimes guide](https://tynd.kvnpetit.com/docs/runtimes).

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

Every release is built by `.github/workflows/build-host.yml` with matching `actions/attest-build-provenance` attestations. The workflow runs only on tagged commits, copies the binaries into the platform sub-packages, and `npm publish`es each in lockstep.

## License

Apache-2.0
