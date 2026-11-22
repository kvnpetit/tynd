# Tynd Plugins

Canonical CLI and documentation metadata lives in [`catalog.json`](./catalog.json).

Tynd separates plugins by ownership and runtime role:

| Kind | Location | Owner | Purpose |
|---|---|---|---|
| Official plugins, host-backed | `plugins/host/*` | Tynd core team | Thin facades over optional native host APIs. |
| Official plugins, standalone | `plugins/standalone/*` | Tynd core team | Higher-level plugins with their own behaviour. |
| Community plugins | external package/repository links | Third-party maintainers | Community or vendor plugins resolved through npm, git, or repository links. |

Every capability has exactly one implementation boundary:

- **Host API** — native implementation lives in `crates/host` behind a Cargo
  `api-*` feature. Only `acl` and `log` are always compiled; every other host
  API is exposed through a `plugins/host/*` package (`tynd add <name>`).
- **Installable plugin** — implementation lives in an official or community plugin
  and enters an app only when listed in `tynd.config.ts`.

Host-backed plugins do not duplicate implementations. They select and type a
host API feature. The other official plugins own their Bun/TypeScript or Rust
implementation.

`packages/*` is reserved for npm packages such as `@tynd/core`, `@tynd/cli`,
`@tynd/plugin-sdk`, and `@tynd/host-src` (host sources compiled into each app);
`npm/*` holds the `@tynd/tooling` binary-carrier packages. Rust-only crates live
under `crates/*`.

Community plugins are intentionally not stored under this repo as local app code.
They are linked as dependencies from their own repositories. A community
manifest omits `tynd.kind` (reserved for official plugins) and may only declare
`apis`, `native` (`path`, `crateName`, `features`) and `backend`:

```json
{
  "tynd": {
    "apis": ["fs"],
    "native": {
      "path": "./native",
      "crateName": "acme-plugin-example",
      "features": ["inference"]
    }
  }
}
```
