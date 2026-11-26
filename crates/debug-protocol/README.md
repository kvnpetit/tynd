# tynd-debug-protocol

JSON-RPC debug protocol transport and taps shared by the Tynd host and tooling.

## What it is

This crate owns the wire format, framing, transport loop and debug taps of the
Tynd debug protocol. The host-specific RPC methods (window list, eval, click,
screenshot, ...) live in `tynd-host` (`src/debug/methods`), not here.

## Modules

- `protocol` — JSON-RPC 2.0 envelopes (`Request`, `Response`, `Notification`,
  `RpcError`) and `read_frame` / `write_frame` / `encode`. Frames are a 4-byte
  little-endian length prefix plus a JSON body, capped at `MAX_FRAME_BYTES`
  (16 MiB).
- `server` — `spawn_accept_loop` listens on an `Endpoint` (`Unix` path: a Unix
  socket, or a named pipe on Windows; or `Tcp` loopback port) and routes each
  request through a host-provided `MethodRouter`. A method returns a
  `CallOutcome`: a result, an error, or a `SubscriptionHandle` that streams
  notifications until `unsubscribe`. Single client by contract: a second
  connection gets an "already attached" error and is closed.
- `ipc_tap` — broadcasts IPC frames (`IpcFrame`) to subscribers and keeps a
  recent-frames buffer. Costs one atomic load when nobody is attached.
- `log_tap` — `DebugLogLayer`, a `tracing` layer that mirrors log events to
  subscribers and keeps a bounded ring for `app.logs.tail`.

## How Tynd uses it

`tynd-host` starts the server when `TYND_DEBUG_SOCKET` or `TYND_DEBUG_PORT` is
set, which `tynd dev` does. Packaged apps refuse to start it unless they were
built with `tynd build --debug-protocol`. `tynd-mcp` (in `crates/cli`)
reuses this crate's framing as a client; `@tynd/test` speaks the same protocol
from TypeScript.

See [Debugging](https://kvnpetit.github.io/tynd/docs/v1.0/guides/debugging).

## Validation

```bash
cargo test -p tynd-debug-protocol
```

## License

Apache-2.0
