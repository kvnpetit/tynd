# tynd-host

Rust host for Tynd desktop apps — webview lifecycle, OS API dispatch, and IPC boundary for Bun backends.

This crate owns:

- **Webview initialization** via wry (Chromium / WebKit / WebView2) + tao window management
- **OS API dispatch** — 30+ native OS integrations (fs, http, dialog, process, etc.)
- **IPC wire format** — `window.__tynd__` shim, devalue codec, call/yield/event round-trip
- **Binary packing** — TYNDPKG format reader for embedded assets and sidecars
- **Plugin system** — trait-based `Arc<dyn TyndPlugin>` with dispatch hooks
- **Public facade** — the entry points the runtime and custom hosts call

## Module layout

```
src/
├─ lib.rs              Public facade: run_app, pre_run, cleanup, init_tracing,
│                      dispatch_api, set_dev_mode, log macros
├─ api.rs              Host API table (`host_apis!`, one row per API), the
│                      `mod` lines of `api/`, dispatch, shutdown order
├─ api/                One module per host API (fs/, http/, process/, updater/,
│                      vault/, power/, deep_link/, menu, tray, window_cmd, ...)
│                      plus shared helpers (progress, registry, watch_refs)
├─ app.rs              Tao event loop, window registry, user events
├─ app/                Bootstrap, primary/secondary/child webviews, window
│                      commands and events, backend bridge (runtime.rs)
├─ ipc.rs, ipc/        Renderer IPC: decoding and routing (dispatch), call
│                      pool, errors, events, call owner
├─ security.rs, security/  ACL engine (acl), path scopes (path_scope), policy
│                      authority (policy)
├─ webview.rs, webview/    WebView builder, IPC handlers, navigation guard,
│                      `tynd://` schemes (scheme, scheme_bin), JS shim (shim)
├─ platform.rs, platform/  OS helpers: color scheme, icons, local listener,
│                      Windows security descriptors
├─ debug.rs, debug/    Debug protocol (feature `debug-protocol`)
├─ plugins.rs, plugins/    Native plugin registry (feature `plugins`)
├─ embed.rs            TYNDPKG unpacker + section reader
├─ observability.rs    Tracing, OpenTelemetry and Sentry setup
├─ cleanup.rs          Exit-time cleanup
├─ signals.rs          Termination signals
├─ window.rs           Window helpers
└─ util.rs             Small shared helpers
```

Every module is `pub(crate)` except `embed`, `observability` and `plugins`.
A module with children is `foo.rs` + `foo/`; there is no `mod.rs`. Tests of
the largest files live in a sibling `foo/tests.rs`.

## Dispatch conventions

### Standard (30+ modules): `dispatch(method, args) -> Result<Value, String>`

Every OS API module exports a dispatch function. Central `api::dispatch` (exported as `tynd_host::dispatch_api`) routes `(api, method, args)`:

```rust
api::dispatch("fs", "readText", json!({"path": "/tmp/hello.txt"}))
```

### Typed (process.rs): `#[tynd::handler(api = "process")]`

Strongly-typed method signatures with automatic serde deserialization. The macro generates a dispatch function next to the impl block, enabling zero-boilerplate per-method extraction and JSDoc capture for the gen-types pipeline.

## OS APIs

| Module | Feature | Purpose |
|--------|---------|---------|
| acl (`security/acl`) | (always-on) | RBAC allow-list validation |
| app | (always-on) | Window ops |
| autolaunch | api-autolaunch | auto-launch on startup |
| biometric | api-biometric | Touch ID / Windows Hello |
| clipboard | api-clipboard | Clipboard read/write |
| compute | api-compute | SubtleCrypto (SHA2, BLAKE3, Ed25519, AES-GCM, PBKDF2) |
| dialog | api-dialog | File/folder dialogs |
| fs | api-fs | File I/O + watchers |
| http | api-http | HTTP client |
| keyring | api-keyring | OS credential storage |
| log | (always-on) | Structured logging |
| network | api-network | Interface enumeration |
| notification | api-notification | Desktop notifications |
| power | api-power | Sleep/wake + idle tracking |
| process | api-process | exec/spawn + streaming |
| screenshot | api-screenshot | Screen capture |
| shell | api-shell | Open URL / file |
| shortcuts | api-shortcuts | Global hotkeys |
| sidecar | api-sidecar | Bundled binary |
| single_instance | api-single-instance | File lock + IPC |
| store | api-store | Persistent JSON |
| system_stats | api-system-stats | CPU/memory/disk |
| theme | api-theme | Dark/light mode |
| updater | api-updater | Signed self-update: binary + bsdiff on Windows/Linux, transactional signed `.app` swap on macOS |
| vault | api-vault | AES-GCM-SIV storage |
| websocket | api-websocket | WebSocket + async |
| window_state | api-window-state | Window geometry |
| deep_link | api-deep-link | URL schemes |
| keys | api-keys | Key intercept |
| cli | api-cli | argv parsing |
| apple_script (macOS) | api-apple-script | osakit |

## Feature flags

### Preset builds
- **Default (`default = []`)** — the minimal core: the window/webview,
  `tynd://`, IPC and lifecycle code plus the `acl` and `log` APIs. These are
  always compiled; the renderer capability check cannot be built out.
- **`api-all`** — explicit full-capability opt-in for tests, debug builds, or
  apps that knowingly want every native OS API in the host.

### Custom builds
```bash
cargo build --no-default-features --features api-fs,api-http,api-process
```

### Plugin-first APIs
Everything outside the core stays out of the preset; each API is fronted by
its `@tynd/plugin-<name>` package. Keep these capabilities out of the preset and wire them through
custom builds/plugins when an app declares them: CLI argv, clipboard, compute,
dialog, key intercept, menu, multiwindow, shell, single-instance, store, theme,
tray, autolaunch, biometric,
filesystem, host HTTP, WebSocket client, notifications, network interfaces,
power/battery/idle, process/sidecar, screenshot, shortcuts, updater, vault,
deep-link, AppleScript, enterprise TLS, window state,
system stats, native plugins, and observability.

### Observability (opt-in)
- `obs-otel` — OpenTelemetry export
- `obs-sentry` — Sentry error capture

## Plugin system

Plugins loaded from CLI-generated workspace, registered at startup. Dispatch routed through `plugins::dispatch` with args ownership to avoid cloning.

```rust
pub struct MyPlugin { }

impl TyndPlugin for MyPlugin {
    fn dispatch(&self, method: &str, args: Value) -> Result<Value, String> {
        match method {
            "customMethod" => { /* ... */ },
            _ => Err(format!("Unknown method: {}", method))
        }
    }
}

register_plugin!(MyPlugin { });
```

## IPC boundary

`window.__tynd__` shim (injected via `with_initialization_script`):

```ts
window.__tynd__.call(api, method, args)     // => Promise<value>
window.__tynd__.on(event, listener)         // Subscribe
window.__tynd__.off(event, listener)        // Unsubscribe
window.__tynd__.osCall(cmd)                 // Window command
window.__tynd__.postMessage(json)           // Raw IPC
```

Call IDs use UUIDv7. Wire format uses the devalue codec (Date/Map/Set/BigInt/cycles) with JSON fallback.

## Public surface

```rust
tynd_host::run_app(..)        // event loop
tynd_host::pre_run()          // early exit of a forwarded deep-link launch
tynd_host::cleanup()          // exit-time cleanup
tynd_host::init_tracing(..)   // logging
tynd_host::dispatch_api(..)   // host API call on behalf of the backend
tynd_host::set_dev_mode(..)
tynd_host::embed::*           // TYNDPKG reader
tynd_host::plugins::{register, register_with_permissions}  // feature `plugins`
```

The Tynd runtime imports from this crate. Native plugin support is
compiled only with the `plugins` feature; plugin authors depend on
`tynd-plugin-sdk` directly.

## Validation

Check the full host feature set with:

```bash
cargo check --all-features
cargo clippy --all-features -- -D warnings
```
## Cross-platform notes

- **Windows** — Win32 DWM backdrop (Mica/Acrylic), Credential Manager keyring
- **macOS** — Keychain keyring, NSWorkspace power events, InfoPlist embedding
- **Linux** — Secret Service keyring (DBus), systemd-logind power events
- **Shared** — Consistent `api::dispatch` across all platforms

## License

Apache-2.0
