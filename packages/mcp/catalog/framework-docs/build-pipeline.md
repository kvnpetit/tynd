---
title: Build Pipeline
description: "How native Rust tooling develops, launches, builds, and packages a TypeScript Tynd application."
---

# Build Pipeline

Tynd separates three execution paths instead of hiding builds behind launch commands:

| Command | Frontend | Backend | Native result |
|---|---|---|---|
| `tynd dev` | Framework dev server/direct assets | Source bootstrap watched and reloaded | Development host process |
| `tynd start` | Existing configured/detected output | Existing cached bundle | Existing host process |
| `tynd build` | Production build/direct bundle | Fresh minified Bun bundle | Packed executable, optional installers |

The npm `@tynd/cli` package only resolves and launches the native executable. Config stays TypeScript-friendly; orchestration runs in Rust.

## Development

`tynd dev`:

1. loads merged config and dotenv inputs;
2. detects and starts the frontend development command;
3. waits up to 30 seconds for `devUrl` when one exists;
4. writes a small backend bootstrap that registers configured JavaScript plugins and runtime/updater globals;
5. resolves the pinned Bun executable and compiles the Rust host package generated in `.tynd/build` with the Cargo `dev` profile (skipped when its inputs did not change since the last successful dev build);
6. supervises frontend, host, and relevant backend/plugin/config inputs.

Frontend HMR remains owned by Vite, Angular, Parcel, Rsbuild, Webpack, or the configured command. Compatible backend changes send `reload` through the host's control channel without recreating the native window. A changed host/config plan requires a `tynd dev` restart.

## Cache-only start

`tynd start` performs no frontend build, backend build, watcher, host build, dependency install, or pack step. It runs the release host compiled by the last `tynd build`.

It accepts only the production backend artifact `.tynd/cache/bundle.dist.js`, plus the configured frontend directory when it exists, otherwise the detected framework output. Missing output, including a missing release host, fails with an instruction to run `tynd build`.

## Production build

`tynd build`:

1. runs the detected frontend production command when applicable;
2. bundles the generated backend entry to `.tynd/cache/bundle.dist.js` with Bun (`--target=bun --minify`);
3. copies a `frontendDir` that has no detected build tool and injects runtime config;
4. compiles the Rust host package generated in `.tynd/build` with the Cargo `release` profile;
5. invokes native Rust `tynd-bundler pack`;
6. invokes native installer bundling only when `--bundle <targets>` is requested;
7. applies configured platform signing.

Default raw output:

```text
release/<platform>-<arch>/<app>[.exe]
```

## Generated state

`.tynd/cache` is a staging directory for bootstraps, bundles, frontend assets, and native bundler JSON. `.tynd/build` contains the generated host package (`Cargo.toml`, `src/main.rs`), `Cargo.lock`, the copies of editable native plugins in `plugins/<crate>/`, and the host binaries copied to `bin/debug/` and `bin/release/`. Required `build` steps still run on every invocation.

## Shared host build cache

Cargo compiles the host into a target directory shared by every project on the machine, from source copies shared the same way:

```text
<cache_root>/host/<key>                     Cargo target directory
<cache_root>/host/src/<digest>              copy of the installed @tynd/host-src sources
<cache_root>/host/plugins/<crate>-<digest>  copy of an installed native plugin crate
```

| Platform | `cache_root` |
|---|---|
| Any, when set | `TYND_CACHE_DIR` (made absolute, with native separators) |
| Windows | `%LOCALAPPDATA%\tynd\cache` |
| macOS | `~/Library/Caches/tynd` |
| Linux | `$XDG_CACHE_HOME/tynd`, else `~/.cache/tynd` |

`<key>` is derived from the host sources (the `@tynd/host-src` version, or the path of editable sources), the `rustc --version` line, and the compiler's host target triple. Third-party dependencies, the Tynd crates, and native plugin crates are compiled once per key and reused by every project: the first project's first build takes a few minutes, later projects with the same key compile only their own generated host package and link. A Rust upgrade or a new `@tynd/host-src` version starts a new key. The `dev` and `release` profiles share the same target directory. `tynd clean --cache` only removes a `host` directory holding the `.tynd-cache` marker Tynd writes there, and a build refuses to adopt a non-empty `host` directory without it.

Cargo identifies a local crate by its absolute path, so a crate read from each project's `node_modules` would be recompiled for every project. The build therefore copies those sources into the cache once and compiles the copies:

- `host/src/<digest>` holds the `native` tree of an installed `@tynd/host-src`. Editable sources (a Tynd checkout, a linked package) are compiled in place, without a copy.
- `host/plugins/<crate>-<digest>` holds an installed native plugin crate (one inside `node_modules`) in `crate/`, next to a generated `Cargo.toml` that provides the workspace values the crate inherits (`version.workspace = true`, `tynd-plugin-sdk.workspace = true`, …).
- An editable plugin (linked by `tynd plugin dev`, or any plugin outside `node_modules`) is copied to `.tynd/build/plugins/<crate>` instead, with the same layout. That copy keeps its path and is synchronized on every build: only changed files are rewritten and deleted files are removed, so an edit recompiles the plugin alone rather than starting a new cache copy.

Each `<digest>` covers the content of every copied file, plus the package version for the host sources and the generated workspace manifest for a plugin. A cache copy is written to a temporary directory (`.tmp-*`, removed when left over an hour by an interrupted build), renamed into place, and never modified afterwards: changed sources, including an edit inside `node_modules`, get a new directory. Old copies stay until `tynd clean --cache`.

On Windows, Cargo and the MSVC tools build deep paths under the target directory, and a long one makes the linker or assembler fail (`LNK1104`, `A1009`). When the target directory is longer than 80 characters, builds and `tynd doctor` warn: set `TYND_CACHE_DIR` to a short path such as `C:\tynd-cache`.

After Cargo finishes, the binary is copied to `.tynd/build/bin/<debug|release>/<app>`, which is what `tynd dev`, `tynd start`, and packing use. `tynd doctor` shows the cache path; `tynd clean --cache` removes it.

On Windows a running executable cannot be replaced: stop the app before rebuilding, or the copy fails with `replace … (is the app still running?)`.

In CI, set `TYND_CACHE_DIR` to a path inside the workspace and cache that directory.

```bash
tynd clean --dry-run
tynd clean --yes
```

`clean` removes `.tynd/cache`, `.tynd/build`, and `release` only; add `--cache` to also remove the shared host build cache.

## Native package

The pack step combines:

- compiled release Rust host;
- exact Bun executable and version;
- bundled backend;
- frontend assets;
- configured sidecars;
- optional fixed WebView2 runtime;
- package metadata.

The payload uses the [TYNDPKG format](https://tynd-apps.github.io/tynd/docs/v1.0/concepts/tyndpkg).

## Installers and signing

Supported current-host formats:

| Host OS | Targets |
|---|---|
| macOS | `macos`, `dmg` |
| Linux | `deb`, `rpm`, `appimage` |
| Windows | `nsis`, `msi` |

Cross-compilation is rejected. Format-specific prerequisites are resolved when that format runs; no broad registry/WebView/toolchain preflight precedes ordinary commands.

Windows signing invokes `signtool` from `PATH` for eligible executable/installer files. macOS signing/notarization uses the Rust bundler. A macOS `.app` update archive is named `<binary>-<version>-macos.zip`.

## Next

- [Bundling](https://tynd-apps.github.io/tynd/docs/v1.0/guides/bundling)
- [Code Signing](https://tynd-apps.github.io/tynd/docs/v1.0/guides/code-signing)
- [Security](https://tynd-apps.github.io/tynd/docs/v1.0/concepts/security)