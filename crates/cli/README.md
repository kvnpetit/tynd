# tynd-cli

The native `tynd` CLI and `tynd-mcp` server for the Tynd framework.

## What it is

`tynd-cli` is the whole developer-tool implementation: command parsing,
config loading, project orchestration, custom host builds, release tooling and
the Model Context Protocol server. It builds two executables:

- `tynd` (`src/bin/tynd.rs`) — the project CLI (`tynd create`, `dev`, `build`,
  `test`, `updater publish`, ...).
- `tynd-mcp` (`src/bin/tynd-mcp.rs`) — the MCP server (`serve` over stdio) plus
  `call` and `tap` subcommands for one-off debug-bridge calls and event streams.

The npm packages `@tynd/cli` and `@tynd/mcp` only ship launchers; the binaries
come from `@tynd/tooling-<os>-<arch>`. Packaging itself runs in the separate
`tynd-bundler` executable (`crates/bundler`), which `tynd build --bundle`
calls (override its path with `TYND_BUNDLER_BIN`). The JSON requests sent to it
are the types of `tynd-bundle-contract` (`crates/bundle-contract`).

## Layout

| Module | Role |
|---|---|
| `args` | clap definitions of every command and flag (`Cli`, `McpCli`) |
| `commands` | `mod.rs` dispatches to one handler module per command group: `project` (create, init, dev, start, build, validate, doctor, info, size, explain, clean, upgrade, config show; `project/tsconfig.rs` patches tsconfigs), `add` (add, remove), `plugins`, `updater` (keygen, sign, publish, release, server), `test`, `report`, `profile`, `completions` |
| `config` | `tynd.config` loading, `extends`, validation and resolution (`TyndConfig` in `config/model.rs`) |
| `project` | project discovery, platform resolution and the generated custom host (`project/custom_host.rs`, under `.tynd/build`) |
| `process` | child-process helpers |
| `output`, `fsx`, `source_edit` | logging, filesystem helpers, comment-preserving edits of JSON-like sources |
| `mcp` | MCP server, tools, resources, prompts, app discovery and the debug bridge |

`build.rs` compiles the MCP static catalog (framework docs and plugin schemas,
generated into `packages/mcp/catalog` by `bun run gen:mcp-catalog`) into the
binary, and `commands/updater/server.rs` embeds the update-server templates of
`templates/update-server`, so the binaries need no file beside them. The debug
bridge talks to running apps with `tynd-debug-protocol`.

## Build and test

```bash
cargo build --release -p tynd-cli --bins
cargo test -p tynd-cli
```

The `local-socket` feature (default) enables Unix socket and named-pipe
transport for the debug bridge.

See the [CLI reference](https://kvnpetit.github.io/tynd/docs/v1.0/cli) and
[`@tynd/mcp`](../../packages/mcp/README.md).

## License

Apache-2.0
