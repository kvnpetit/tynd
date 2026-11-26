# Testing plugins

How the plugins under `plugins/host/*` and `plugins/standalone/*` are tested.

## Manifest checks

`plugins/manifests.test.ts` runs with `bun test` and validates every
plugin `package.json`:

- `@tynd/*` peer dependencies are optional (`peerDependenciesMeta`), so a frozen install never
  fetches them from the public registry.
- `homepage` is `https://kvnpetit.github.io/tynd/docs/v1.0/api/plugins/<dir>`.
- `@tynd/*` entries in `dependencies`, `devDependencies` and `optionalDependencies` use `workspace:` ranges.
- the `tynd` field exists and its `kind` matches the group directory (host-backed plugins also declare `sdk: true`
  and `apis: ["<dir>"]`).

```sh
bun test plugins/manifests.test.ts
```

## Type checks

Each group has a `tsconfig.json`, run from the repo root as part of `bun run check`:

```sh
tsc --noEmit -p plugins/host
tsc --noEmit -p plugins/standalone
```

## Lint and format

```sh
bunx biome check plugins
```

## Unit tests

Plugin logic is tested with `bun test` and `*.test.ts` files placed next to the code they cover
(for example `plugins/standalone/sql/backend.test.ts`). Backend plugins that use Bun APIs
(`bun:sqlite`, `Bun.serve`) run directly under `bun test`; avoid tests that need a running host.

## Native plugins

The `audio`, `terminal` and `tts` plugins include Rust crates. Test each crate with a scoped Cargo command:

```sh
cargo test -p tynd-plugin-<name>
cargo clippy -p tynd-plugin-<name> -- -D warnings
```

Host-backed plugins are thin facades over host features; their native behaviour is tested in `crates/host`.

## Not covered here

End-to-end checks against a running app (window, webview, IPC) are done through the UI verification flow,
not through plugin unit tests.
