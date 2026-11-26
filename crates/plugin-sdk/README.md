# tynd-plugin-sdk

Shared Rust SDK for Tynd native plugins — implements the TyndPlugin trait, event emission, OS dispatch, and state management.

## Overview

Third-party plugins implement TyndPlugin and export via the register_plugin! macro. Plugins ship their Rust sources in their npm package under native/ and are picked up at build time by tynd build — the CLI wires them into a generated Cargo workspace alongside the host. At runtime the host's plugin dispatcher looks up plugins by name() and delegates each IPC call to TyndPlugin::dispatch.

The SDK is deliberately minimal: no trait objects for events, no ABI versioning (plugins and host compile together), JSON values over serde. The SDK wraps only what's shared across plugins; anything plugin-specific belongs in the plugin crate itself.

## Architecture

### TyndPlugin trait

Every plugin implements the trait with name(), init(), dispatch(), and shutdown() methods. Plugins are stored as Arc<dyn TyndPlugin> in the host registry for:
1. Lock-free plugin lookup - arc-swap so dispatch reads never block on a write lock.
2. Plugin shutdown hooks - shutdown() gives plugins a chance to clean up before host exit.
3. Future-proof for libloading hot reload - Arc<dyn ...> can survive after a libloading::Library is dropped.

Every method takes &self - interior mutability is the plugin's responsibility (parking_lot::Mutex<State>, dashmap::DashMap, arc-swap, etc.).

### PluginCx - Plugin context

Handed to every dispatch call; keeps the surface opaque so future additions don't break plugin crates. Methods:

- emit(event, payload) - Fire-and-forget event to all webviews. Payload is JSON-serialized by the host.
- emitter() -> EmitFn - Clone-able handle on the emit callback. Useful when a plugin spawns background threads that outlive the &PluginCx borrow.
- os_call(api, method, args) -> PluginResult<Value> - Invoke a core OS API (fs, http, dialog, etc.). Denied by default and gated by the host-wired allowed_apis allow-list.
- spawn_interval(name, interval, body) -> PluginResult<BackgroundHandle> - Spawn a background thread that calls body every interval. Thread-creation failures are returned; the handle's Drop signals exit and joins the worker.
- test() -> PluginCx - Build a noop context for unit tests.
- with_dispatch(fn) - Attach the host's OS dispatcher so os_call can route through it. Host-only.
- with_allowed_permissions(permissions) - Restrict to the given api:method selectors (api:* for a whole API).

### PluginError - Typed error enum

Plugin methods return PluginResult<T> = Result<T, PluginError>. Error variants include:

- MissingArg { key } - Required argument was absent.
- InvalidArg { key, detail } - Argument failed to deserialize into the requested type.
- NotPermitted { api, method, allowed } - Plugin tried to reach an OS API outside its allow-list.
- DispatchNotWired - cx.os_call(...) invoked from a context with no host dispatcher.
- UnknownMethod { plugin, method } - Plugin's dispatch fallthrough.
- Custom(String) - Free-form plugin error (format!(...).into() lands here).
- Io / Json - Transparent forwarding from std::io::Error / serde_json::Error.

Designed to be friendly to the existing format!(...).into() idiom - a From<String> / From<&str> impl maps ad-hoc strings to Custom, so plugins written against older Box<dyn Error> keep compiling unchanged.

### Registry<T> - Concurrent handle table

Every plugin that carries per-call state (sql connections, terminal sessions, watcher subscriptions) builds the same DashMap<u64, Mutex<T>> + AtomicU64 trio. Registry<T> lifts that into the SDK so plugins drop ~25 LoC of boilerplate each and inherit a consistent overload-rejection error shape.

Uses dashmap::DashMap (sharded locking) and parking_lot::Mutex (no poisoning, no contention slowdown after a panic).

Methods: insert(value) -> PluginResult<u64>, with(id, f) -> PluginResult<R>, remove(id) -> bool, ids() -> Vec<u64>, len() -> usize, is_empty() -> bool.

### ArgsExt - Typed argument extraction

Extension trait on serde_json::Value that tightens argument validation boilerplate:

- require<T>(key) - Pull a required field, deserialized to T. Returns proper error messages on missing or invalid shapes.
- opt<T>(key) - Pull an optional field. Returns None when absent or fails to deserialize.

### BackgroundHandle - Cancellable background threads

Returned by PluginCx::spawn_interval:

- stop() - Signal the background loop to exit at next check.
- is_running() -> bool - Whether the loop has seen the stop signal yet.
- join() - Stop and block until the worker thread finishes.

Dropping the handle stops the loop and joins the worker. The wait between ticks blocks on a crossbeam_channel recv, so a stop() interrupts immediately — shutdown is bounded only by the time body itself takes.

### Zero-copy dispatch

Optional TyndPlugin::dispatch_rkyv method for high-performance plugins using rkyv-encoded buffers instead of JSON. Opt-in by overriding the method; the #[plugin_command(zero_copy)] marker on the args struct adds the Archive/Serialize/Deserialize derives. The dispatch_rkyv arm is hand-written so the plugin author chooses when the wire-format flip is safe.

Helper functions: rkyv_decode_args, rkyv_encode_result.

### Macros

- register_plugin!(constructor) - Expands to pub fn __tynd_plugin() -> Arc<dyn TyndPlugin>. The CLI wires a call to that function into the host-generated main.rs.
- #[tynd_plugin::commands] - Proc-macro that generates the dispatch method from struct field declarations and #[plugin_command] methods. Eliminates hand-rolled match method { ... } ladders.
- #[tynd_plugin::skip] - Opt out a method from dispatch generation.
- #[plugin_command(zero_copy)] - Mark an args struct to opt into the zero-copy path.
- tynd_plugin::gen_types_bin!() - Generate the fn main() for the plugin's standalone gen-types binary.

### Testing helpers

- test_cx() -> (PluginCx, EventCapture) - Build a context pre-wired for unit tests; every cx.emit(...) is recorded.
- EventCapture - Event recorder with events(), len(), is_empty(), clear() methods.
- unknown_method(plugin, method) -> PluginError - Format the canonical "unknown method" error.

## Use

Plugins depend on the SDK:

```toml
[dependencies]
# Official plugins inherit this dependency from the monorepo workspace.
tynd-plugin-sdk = { workspace = true }
```

Implement the trait and export via native/src/lib.rs (the host's build picks it up automatically).

## Dependencies

- serde / serde_json - JSON value handling.
- thiserror - Error type derivation.
- rkyv - Zero-copy archive format (opt-in via dispatch_rkyv).
- dashmap - Sharded concurrent map (for Registry<T>).
- parking_lot - Fast mutex (for Registry<T>).
- crossbeam-channel - Bounded MPMC channels (for spawn_interval stop signal).
- pollster - Lightweight block_on bridge for the `#[plugin_impl]` async-fn dispatcher.
- tracing - Structured warnings from background-handle teardown (Drop paths can't surface errors otherwise).
- specta / specta_typescript / specta_serde - Type metadata + TypeScript exporter + serde-attr phase hook used by `#[plugin_impl]` / `#[commands]` codegen.

Re-exported publicly as tynd_plugin_sdk::rkyv / ::specta / ::specta_typescript / ::specta_serde so plugin authors don't pin the versions themselves.

## Validation

Run `cargo check -p tynd-plugin-sdk` after changing the native SDK contract.