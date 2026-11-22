# tynd-runtime

The Bun JavaScript runtime for Tynd desktop apps.

## What it is

`tynd-runtime` spawns Bun and manages bidirectional IPC between the Rust host and the JavaScript backend. It ships the Node-compatible Bun API surface and is compiled per app by the Tynd CLI (from `@tynd/host-src`) and bundled inside packaged Tynd apps.

- **JS runtime:** Bun (subprocess, not embedded)
- **Binary size:** ~44–50 MB (zstd-compressed bun binary + host runtime)
- **Best for:** all Tynd backends, including CPU-bound JavaScript and npm native bindings

Designed for every Tynd app backend, with Bun's Node-compatible standard library.

## Architecture

The runtime operates as a **stateless subprocess manager**:

1. **Spawn:** `spawn_bun()` launches the Bun child with stdin/stdout piped
2. **IPC loop:** Two dedicated threads handle message passing:
   - **Stdin forwarder:** coalesces `BackendCall` frames into a single `write_all` syscall to burst through IPC storms
   - **Stdout reader:** decodes `BunMsg` events and forwards them to the app event channel
3. **Reload:** `ReloadHandle` swaps the Bun subprocess in place during dev mode (kills child, spawns new, restarts reader thread) while the host and WebView stay alive
4. **Cleanup:** `ChildSlot` Drop impl guarantees orphaned processes are reaped — critical on Windows (TIME_WAIT on stdout pipes) and long-running dev sessions

Source ownership stays explicit:

- `bun.rs` owns Bun process supervision, reload serialization, worker threads, and host-event routing.
- `bun/wire.rs` owns JSON schemas, bounded framing, config decoding, and outbound call encoding.
- `bun_ipc.rs` owns the Bun IPC frame size cap (frames are newline-delimited JSON).

Startup fails after 30 seconds if the backend never emits `tynd:config`. Bun stderr is always drained into the host logs, including packaged GUI builds.
Each Bun generation owns a separate expected-shutdown marker, so delayed EOF
from an old reader cannot report a false crash against a newer child. An
unexpected exit rejects current and future backend calls with `BACKEND_DEAD`;
successful dev reload replaces the document and resets the shim state. Reload
handles share a serialization lock, preventing concurrent filesystem events
from spawning competing Bun children.

## Message flow

**Host → Bun (stdin):**
```
BackendCall::Typed { id, fn_name, args }  →  WireMsg::Call
BackendCall::Cancel { id }                →  WireMsg::Cancel
BackendCall::Ack { id, n }                →  WireMsg::Ack
BackendCall::Raw(json)                    →  passed verbatim (JSON only)
```

**Bun → Host (stdout):**
```
BunMsg::Config       →  initial window/menu/tray config
BunMsg::Return       →  call result or error
BunMsg::Yield        →  streaming generator frame
BunMsg::Event        →  user emit (devalue-encoded)
BunMsg::EvalInFrontend  →  script injection for webview bridge
```

## IPC wire format

Newline-delimited, UTF-8 JSON lines on stdin/stdout, decoded with `serde_json`.
There is no alternative format and no environment switch.

Max frame size: 8 MiB, in both directions.

## Hot reload (dev mode)

`ReloadHandle` supports swapping the Bun backend without restarting the host:

```rust
// CLI stdin thread (debug mode only)
"reload"           → reload_handle.reload()           // kill Bun, respawn
"reload-frontend"  → reload_handle.reload_frontend_only()  // just reload webview
```

This works because the stdin forwarder buffers frames under a mutex — `ReloadHandle::reload()` atomically swaps the child handle, restarts the reader thread, and clears the `reload_in_progress` flag so the reader doesn't mistake the expected stdout close as a Bun crash.

## Embedded assets

When packed via the CLI, the binary tail contains a `PackReader` stream:

```
bun.version          → Bun release tag (e.g., "v1.0.2")
bun.blake3           → Signed-pack hash of the uncompressed Bun binary
bun.zst              → Bun binary (zstd-compressed)
bundle.js            → Bun-bundled JS entry point
frontend/<path>      → Pre-built React/Vue/etc static files
icon.{png,ico}       → App icon (extracted to temp, loaded by host)
sidecar/<name>       → Optional auxiliary binaries (extracted + chmod +x)
observability.json   → Resolved `observability` config (optional)
debug.flag           → Present only for `tynd build --debug-protocol`
webview2.runtime-path → WebView2 fixed-runtime path (Windows, optional)
```

On first launch, `bun.zst` is decompressed to the user's cache directory:
- Windows: `%LOCALAPPDATA%\tynd\{app}\bun-{version}-{hash}\bun.exe`
- macOS: `~/Library/Caches/tynd/{app}/bun-{version}-{hash}/bun`
- Linux: `~/.cache/tynd/{app}/bun-{version}-{hash}/bun`

Extraction is disk-streamed, size-bounded, hash-verified, and atomically published into a hash-qualified cache directory. Subsequent launches verify the cached binary before use. Packaged apps fail closed if Bun metadata or bytes are missing/corrupt; they never fall back to an unrelated `bun` from `PATH`. In dev/start mode, the CLI validates `.bun-version` and passes its own executable through `TYND_BUN_PATH`; only direct low-level launches of `tynd-runtime` fall back to `bun` on `PATH`.

## Feature flags

| Flag | Effect |
|------|--------|
| `preset` (default) | Links the minimal host core (always compiled, with `acl` + `log`) and enables only the host debug features (`debug-protocol`, `debug-screenshot`). No plugins bundled; every other native API is added through its `@tynd/plugin-*` package in `tynd.config.ts`. |

## Custom builds

The preset only includes the host core (`acl` + `log`) and NO plugins. Apps
request host APIs and native plugins in `tynd.config.ts` and the CLI generates
a wrapper crate:

```rust
// .tynd/build/src/main.rs (generated, simplified)
fn main() {
    assert!(
        tynd_host::plugins::register_with_permissions(
            tynd_plugin_terminal::__tynd_plugin(),
            Some(vec![/* plugin permissions resolved by the CLI */]),
        ),
        "native plugin registration failed: {}",
        stringify!(tynd_plugin_terminal),
    );
    tynd_runtime::run();
}
```

Then builds with:
```bash
cargo build -p tynd-runtime \
  --no-default-features \
  --features "tynd-host/api-fs,tynd-host/api-system-stats"  # or any subset
```

This keeps binary size lower for apps that don't need every OS API.

## Validating feature combinations

```bash
# Default feature set
cargo check -p tynd-runtime

# Custom build with a subset of OS APIs
cargo check -p tynd-runtime --no-default-features \
  --features "tynd-host/api-fs"
```
## Related packages

- [`@tynd/core`](https://www.npmjs.com/package/@tynd/core) — Backend API + frontend client
- [`@tynd/cli`](https://www.npmjs.com/package/@tynd/cli) — `tynd create / dev / build / start`
- [`tynd-host`](../host) — Shared Rust runtime (OS APIs, plugins, windowing)

## License

Apache-2.0
