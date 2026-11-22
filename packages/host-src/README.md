# @tynd/host-src

Generated Rust source mirror of the Tynd host, packaged as an npm module.

This package contains the self-contained Rust host, Bun runtime, and internal
crate sources that `@tynd/cli` compiles with cargo into every app's host.

## Contents

- `native/` — standalone Cargo workspace containing `tynd-host`,
  `tynd-runtime`, and vendored internal crates. Workspace dependency references
  are flattened to explicit versions or local paths.
- `scripts/sync.ts` — regenerates `native/` from `crates/host/`,
  `crates/runtime/`, the crates they depend on, and the workspace `Cargo.toml`.

## Maintenance

Run these commands from `packages/host-src`:

```bash
bun run sync     # regenerate native/
bun run lock     # regenerate native/Cargo.lock
bun run prepack  # run the package's prepack sync
```

`tynd create` and `tynd init` add this package to the app's `devDependencies`
(it is also a dependency of `@tynd/cli`). `tynd dev` and `tynd build` compile
this source workspace with cargo into the app's host, with only the host APIs
and native plugins the config selects.