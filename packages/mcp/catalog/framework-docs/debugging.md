---
title: Debugging
description: "Debug frontend WebViews, Bun backend code, native logs, and authenticated development IPC with current Rust tooling."
---

# Debugging

A Tynd app has three observable surfaces: WebView frontend, Bun backend, and Rust host/tooling.

## Frontend DevTools

```ts
import { tyndWindow } from "@tynd/core/client";

await tyndWindow.openDevTools();
await tyndWindow.closeDevTools();
```

Use normal Console, Elements, Network, Sources, and Performance panels. Tynd assets/IPC use native schemes such as `tynd://` and `tynd-bin://`, not a production loopback HTTP server.

## Bun inspector

Use native CLI flags:

```bash
tynd dev --inspect
tynd dev --inspect-brk
```

`--inspect-brk` waits before user backend code. Connect with a Bun-compatible inspector client.

## CLI logs

```bash
tynd --verbose dev
tynd --log-format json --log-file .tynd/cli.log dev
tynd --quiet start
```

Global flags may appear before or after the subcommand. `--verbose` enables debug-level CLI records; `--quiet` suppresses logger output. `--log-file` appends the rendered records and rejects a symbolic-link target.

Application/backend output still uses the host process streams. For a packed executable, redirect its stderr using normal OS shell facilities:

```bash
./release/linux-x64/my-app 2> app.log
```

## Native MCP/debug bridge

`tynd dev` exposes a local authenticated debug endpoint unless `--no-debug` is used. The native Rust `tynd-mcp` server discovers a live app without a project-specific TypeScript MCP process:

```bash
bunx @tynd/mcp serve
bunx @tynd/mcp call app.info
bunx @tynd/mcp call ipc.recent --args '{"n":20}'
bunx @tynd/mcp tap
```

Evaluation and external-launch capabilities remain opt-in through MCP flags. Discovery defaults to loopback endpoints. See [`tynd-mcp`](https://tynd-apps.github.io/tynd/docs/v1.0/cli/mcp) for the flags and environment variables.

## Project diagnostics

```bash
tynd info
tynd validate
tynd doctor
tynd doctor --state
```

- `info`: Tynd/Bun versions and resolved host/bundler paths.
- `validate`: config/backend/frontend/package inputs; `--ci` additionally resolves Bun pin, host, and bundler.
- `doctor`: config, entries, Bun, host, bundler, plus optional generated-state sizes.

These commands do not query the Windows registry, probe WebView availability, or perform a broad platform-tool preflight.

## Common symptoms

### Frontend never becomes ready

Tynd waits 30 seconds for configured/detected `devUrl`. Check the frontend command directly, then set `devCommandArgs` and `devUrl` explicitly when detection does not match the project.

### Backend change asks for restart

Config/plugin/API changes can alter the native host plan. Restart `tynd dev`; only compatible source changes use the live reload channel.

### RPC call fails or hangs

- Confirm the backend export name and `app.start()` call.
- Check Bun/backend stderr.
- Inspect recent/failed IPC through `tynd-mcp` while `tynd dev` is running.
- Confirm the frontend proxy uses the expected backend type.

### OS call is denied

The API may be absent from the compiled host (its plugin is not declared) or a native plugin permission may not include `api:method`. Inspect config/plugin metadata; do not broaden permissions beyond the needed method.

### Packaged app has no MCP endpoint

The discovery/debug socket is a development feature. The debug protocol is **disabled in packaged apps** unless they are built with `tynd build --debug-protocol`. Use that flag only for diagnostic builds; for shipped-user support build a dedicated diagnostic channel/log flow instead of exposing evaluation in production.

## Related

- [Troubleshooting](https://tynd-apps.github.io/tynd/docs/v1.0/troubleshooting)
- [Observability](https://tynd-apps.github.io/tynd/docs/v1.0/concepts/observability)
- [IPC Model](https://tynd-apps.github.io/tynd/docs/v1.0/concepts/ipc)