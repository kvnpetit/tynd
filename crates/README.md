# Rust crates

This directory owns Rust-only workspace crates.

- `host` contains the native WebView host.
- `runtime` contains the Bun-backed executable.
- `cli` contains the native `tynd` CLI and `tynd-mcp` server.
- `bundler` contains `tynd-bundler`, the installer, signing, patch and pack executable the CLI
  runs; `bundle-contract` holds the JSON request types both sides compile against.
- `plugin-sdk` and `plugin-macros` are the Rust plugin SDK; `@tynd/plugin-sdk` ships a generated
  mirror of both.
- Remaining crates provide shared protocols, policies, macros, and binary formats.

Rust code that is an implementation detail of an npm package stays beside that package:

- `plugins/standalone/*/native`

This boundary keeps Cargo-only crates out of `packages/`, which is reserved for JavaScript/npm
workspaces and their colocated native implementation details.
