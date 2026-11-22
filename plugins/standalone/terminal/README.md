# @tynd/plugin-terminal

Embedded PTY for Tynd apps. Spawn shells, stream their output as bytes,
resize on the fly, kill when done. Native Rust via
[`portable-pty`][pp] — ConPTY on Windows, `forkpty` on POSIX.

**Requires** a Rust toolchain; Tynd compiles the plugin crate into the app's
host automatically. Apps that do not declare it do not carry `portable-pty`
and its dependency tree.

## Install

```sh
tynd add terminal
```

`tynd add` installs the package and lists it in `plugins` in `tynd.config.ts`:

```ts
// tynd.config.ts
export default {
  plugins: ["@tynd/plugin-terminal"],
  backend: "backend/main.ts",
  frontendDir: "dist",
}
```

```ts
// backend/main.ts
import { app } from "@tynd/core"

app.start({ window: { title: "Terminal" } })
```

## Use

```ts
import { terminal } from "@tynd/plugin-terminal"

const session = await terminal.spawn({
  shell: "/bin/bash",       // or "cmd.exe", "pwsh.exe", etc. Default: $SHELL / %COMSPEC%
  args: ["-l"],
  cwd: "/home/user",
  env: { LANG: "en_US.UTF-8" },
  cols: 120,
  rows: 32,
})

session.onData((chunk) => xtermInstance.write(chunk))
session.onExit((code) => console.log("shell exited with", code))

await session.write("ls -la\n")
await session.resize(140, 40)
// ...later
await session.kill()
```

## API

| Export | Purpose |
|---|---|
| `terminal.spawn(opts?)` | Open a PTY + child process. Returns `TerminalHandle`. |
| `terminal.list()` | IDs of currently open sessions. |
| `handle.write(data)` | Bytes to stdin (string auto-encoded as UTF-8). |
| `handle.resize(cols, rows)` | Update the PTY size — send SIGWINCH to the child. |
| `handle.kill()` | Close the session + stop the reader thread. Silences the normal `onExit` event. |
| `handle.onData(fn)` | Subscribe to output chunks. Returns unsubscribe. |
| `handle.onExit(fn)` | Subscribe to process exit (`null` when unable to determine code). Fires once. |

Bytes move through IPC as base64 strings; the client helper encodes/
decodes so consumer code stays in `Uint8Array`.

## Limits

| Constraint | Value |
| ---------- | ----- |
| Max concurrent sessions | 64 |
| Reader backlog before chunk drop | 256 |
| `kill()` semantics | SIGKILL via `ChildKiller`; suppresses synthetic `terminal:exit` event; idempotent |

The session cap throws on overflow. The reader thread drops output
chunks (logged) when a stuck consumer falls more than 256 callbacks
behind so a hung JS handler can't OOM the host.

## Pairing with xterm.js

Most apps render PTY output through [xterm.js][xt]:

```ts
import { Terminal as Xterm } from "xterm"
import { terminal } from "@tynd/plugin-terminal"

const view = new Xterm({ cols: 120, rows: 32 })
view.open(document.querySelector("#term")!)
const session = await terminal.spawn()
session.onData((chunk) => view.write(chunk))
view.onData((s) => session.write(s))
view.onResize(({ cols, rows }) => session.resize(cols, rows))
```

[pp]: https://github.com/wez/wezterm/tree/main/pty
[xt]: https://xtermjs.org/

---

See also: [tynd-apps/plugins](../README.md) — plugin index.
