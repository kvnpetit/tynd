# tynd-bundle-contract

JSON request types exchanged between the `tynd` CLI and the `tynd-bundler`
executable.

## What it is

`tynd build` does not link the installer and signing stack: it writes a JSON
request under `.tynd/cache/` and runs `tynd-bundler <subcommand> <file>`. This
crate defines those requests once, so the writer (`crates/cli`) and the reader
(`crates/bundler`) cannot drift apart. It depends only on `serde`.

| Subcommand | Request |
| --- | --- |
| `bundle` | `BundleConfig` |
| `codesign` | `CodesignConfig` |
| `pack` | `PackConfig` |

Every type rejects unknown fields. `patch`, `archive-app` and `dmg-from-app`
take command-line flags and have no request type. The bundler answers on
stdout with one artifact path per line.

## Changing the contract

Field and variant names are the wire format. Both binaries ship together in
`@tynd/tooling-<os>-<arch>`, so a change only has to compile on both sides;
update `src/tests.rs` when a wire name changes on purpose.
