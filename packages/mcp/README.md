# @tynd/mcp

[![License](https://img.shields.io/badge/license-Apache%202.0-blue.svg)](LICENSE)
[![CI](https://github.com/tynd-apps/tynd/actions/workflows/ci.yml/badge.svg)](https://github.com/tynd-apps/tynd/actions/workflows/ci.yml)
[![npm](https://img.shields.io/npm/v/@tynd/mcp.svg)](https://www.npmjs.com/package/@tynd/mcp)
[![MCP](https://img.shields.io/badge/MCP-compatible-8A2BE2.svg)](https://modelcontextprotocol.io)

Drive a running Tynd development app from an AI agent with no
project-specific MCP implementation.

`@tynd/mcp` launches the native Rust `tynd-mcp` server. It attaches to a live
Tynd app and exposes screenshots, DOM snapshots, WebView interaction, IPC
tracing, plugin metadata, capability checks, resources, and guided prompts.

## Distribution

The npm package publishes:

- a small launcher for `tynd-mcp`;
- 14 static framework documents;
- 7 generated plugin schemas.

`@tynd/tooling` selects the matching macOS, Linux, or Windows package for x64
or ARM64. Protocol handling, discovery, the debug bridge, tools, resources,
and prompts execute in Rust. There is no TypeScript server bundle, install
script, or binary download.

Bun remains the runtime for the application being inspected. It does not
implement the MCP server.

## Run

```bash
bunx @tynd/mcp serve
```

For local development from the repository root:

```bash
cargo run -p tynd-cli --bin tynd-mcp -- serve
```

## MCP client configuration

Use `bunx` as the command and `["@tynd/mcp", "serve"]` as its arguments:

```json
{
  "mcpServers": {
    "tynd": {
      "command": "bunx",
      "args": ["@tynd/mcp", "serve"]
    }
  }
}
```

The server speaks MCP over stdio.

## Native CLI modes

`serve` is the default MCP protocol mode. Two diagnostic modes are also
available:

```bash
# Invoke one allow-listed debug bridge method.
bunx @tynd/mcp call app.info
bunx @tynd/mcp call ipc.recent --args '{"n":20}'

# Print IPC frames until interrupted.
bunx @tynd/mcp tap
```

Use `bunx @tynd/mcp --help` and
`bunx @tynd/mcp <serve|call|tap> --help` for authoritative flags.

## How it works

1. `tynd dev` writes an authenticated discovery entry in the OS-standard Tynd
   debug directory.
2. `tynd-mcp` selects the newest live matching app.
3. It connects to the local debug endpoint.
4. Native methods are exposed as MCP tools, resources, and prompts.

When no live app is running, bundled framework documents, generated plugin
schemas, and known-error guidance remain available.

## Tool surface

- `app.info`, `app.launch`, `app.quit`, `app.restart`, `app.logs.tail`
- `window.list`, `window.info`, `window.manage`, `screenshot.window`,
  `screenshot.thumbnail`
- `webview.snapshot`, `webview.find_element`, `webview.console_logs`,
  `webview.network_requests`
- `webview.click`, `webview.type`, `webview.fill`, `webview.press_key`,
  `webview.scroll`, `webview.hover`, `webview.drag`, `webview.evaluate`
- `ipc.tap_start`, `ipc.tap_stop`, `ipc.recent`, `ipc.failed`, `ipc.replay`
- `plugin.list`, `plugin.docs`, `plugin.command_catalog`
- `capability.list`, `capability.check`, `capability.suggest_grant`
- `runtime.snapshot`

Resources include framework docs, plugin schemas, known errors, recent logs,
IPC frames, and screenshots. Prompts cover setup, IPC failures, WebView
errors, permissions, visual regression, and stack traces.

## Security

Evaluation and external launch tools are disabled by default. Enable only the
capability needed by the MCP client:

```bash
bunx @tynd/mcp serve --allow-eval
bunx @tynd/mcp serve --allow-launch
```

Discovery defaults to loopback endpoints and uses per-session authentication
material created by the running Tynd host.

## Related packages

- [`@tynd/tooling`](https://www.npmjs.com/package/@tynd/tooling) — native
  executable resolver and platform packages
- [`@tynd/cli`](https://www.npmjs.com/package/@tynd/cli) — native project CLI
  and typed config
- [`@tynd/core`](https://www.npmjs.com/package/@tynd/core) — application API
- [`@tynd/plugin-sdk`](https://www.npmjs.com/package/@tynd/plugin-sdk) —
  plugin authoring SDK

## License

Apache-2.0. See [LICENSE](./LICENSE).
