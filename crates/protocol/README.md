# tynd-protocol

Pure protocol types shared by the Tynd native host and the Bun runtime.

## What it is

`tynd-protocol` owns the serializable contracts that cross the host/runtime
boundary. It deliberately avoids `wry`, `tao` and OS APIs, so the runtime can
depend on it without pulling in the windowing stack.

## Modules

### `runtime` — host/backend messages

- `BackendBridge` — everything `tynd_host::run_app` needs to talk to the
  backend: the startup `BackendConfig`, a `call_tx` sender of `BackendCall` and
  an `event_rx` receiver of `BackendEvent`.
- `BackendCall` — host to backend: `Typed` call, `Cancel`, `Ack` (stream flow
  control), `HostResult`, or a `Raw` JSON frame.
- `BackendEvent` — backend to host: `Return` (with `BackendReturnPayload`),
  `Yield` (stream chunk), `Emit`, `EvalInFrontend`, `Reload`, `Error`.
- `BackendConfig` — what the backend sends at startup: window, menu, tray,
  `security`, `capabilities`, dev URL, frontend dir, icon and
  `quit_on_last_window_closed`.
- `WINDOW_EVENT_FN` / `WindowEventKind` — reserved call that reports window
  lifecycle (`Created`, `Closed`) to the backend.

### `window` — window and menu shapes

`WindowConfig` and its enums (`ColorSpace`, `PreferredFps`,
`HardwareAcceleration`, `GpuPreference`, `GpuVendor`), the menu definitions
(`MenuItemDef` with `ItemDef`, `CheckboxDef`, `RadioDef`, `SubmenuDef`,
`MenuRole`) and `TrayConfig`. They mirror the TypeScript types of `@tynd/core`.

## How Tynd uses it

`tynd-runtime` (`src/bun.rs`, `src/bun/`) decodes Bun's stdout frames into
`BackendEvent`s and encodes `BackendCall`s for its stdin. `tynd-host` consumes
the `BackendBridge` in its event loop and builds windows, menus and the tray
from `BackendConfig`.

## Validation

```bash
cargo test -p tynd-protocol
```

## License

Apache-2.0
