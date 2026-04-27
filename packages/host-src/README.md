# @tynd/host-src

Rust source tree of the Tynd host, packaged as an npm module.

This package exists so that apps on `runtime: "full-custom"` or `"lite-custom"` can
build a trimmed-down host locally via `cargo`, picking exactly the `api-*`
features they need. Preset runtimes (`full`, `lite`) never install this package —
they pull the prebuilt binaries from `@tynd/host`.

## Contents

- `native/` — self-contained copy of `packages/host-rs/`, with every workspace
  dependency reference flattened to an explicit version so the crate builds
  standalone inside the workspace that the Tynd CLI generates at
  `.tynd/build/`.
- `scripts/sync.ts` — regenerates `native/` from `packages/host-rs/` + the
  workspace `Cargo.toml`. Run via `bun run sync`. Also wired to
  `prepublishOnly`.

## How the CLI uses it

At `tynd build` time, when `runtime` is `full-custom` or `lite-custom`:

1. CLI reads `package.json::tynd.hostSrc` to locate the sources.
2. Copies (or symlinks) `native/` into `.tynd/build/host/`.
3. Generates a `.tynd/build/Cargo.toml` workspace that pulls `host` as a path
   dep with the chosen `api-*` features enabled and `default-features = false`.
4. Runs `cargo build --release` — the resulting binary ships only the modules
   the app actually uses.

You do **not** need to install this package for preset builds.
