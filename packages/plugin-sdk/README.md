# @tynd/plugin-sdk

[![License](https://img.shields.io/badge/license-Apache%202.0-blue.svg)](LICENSE)
[![CI](https://github.com/kvnpetit/tynd/actions/workflows/ci.yml/badge.svg)](https://github.com/kvnpetit/tynd/actions/workflows/ci.yml)
[![npm](https://img.shields.io/npm/v/@tynd/plugin-sdk.svg)](https://www.npmjs.com/package/@tynd/plugin-sdk)

Build [Tynd](https://github.com/kvnpetit/tynd) plugins — JavaScript-only or with native Rust code.

```bash
bun add @tynd/plugin-sdk @tynd/core
```

> `@tynd/core` is a `peerDependency` of this package; Bun ≥1.0 does not auto-install peer deps, so add both explicitly.

## Two flavors of plugins

| Flavor | When to use | Entry files | Native Rust |
|---|---|---|---|
| **Pure JS** | UI helpers, formatters, routers, anything that doesn't need OS access beyond what Tynd already exposes | `backend.ts` (`definePlugin`) + `client.ts` (`createPluginClient`) | No |
| **Native** | OS access not covered by core (clipboard history, OS keychain, PTY, speech), heavy CPU work, FFI | `client.ts` (`defineTypedPlugin`) + generated `<name>.gen.ts` + `native/` Rust crate | Yes |

> Every app's host is compiled with Cargo, so Rust is always required on the dev machine; a native plugin's crate is compiled into that host. Pure-JS plugins add no Rust code. All backend code runs on Bun.

## Getting started — your first plugin

### Step 1. Scaffold

```bash
# from your app or workspace root
tynd plugin new my-plugin               # TypeScript only
tynd plugin new my-plugin --native      # TypeScript + Rust
```

You get (pure JS plugins have `backend.ts`; native plugins have `<name>.gen.ts` and `native/` instead):

```
my-plugin/
├── package.json
├── README.md
├── backend.ts           ← pure JS: methods, run on Bun (definePlugin)
├── client.ts            ← frontend entry (createPluginClient; defineTypedPlugin when native)
├── my-plugin.gen.ts     ← native only: generated command catalog
└── native/              ← native only: Rust crate
    ├── Cargo.toml
    └── src/
        ├── lib.rs
        └── bin/gen-types.rs
```

### Step 2. Define the backend

**Pure JS plugin** — `backend.ts`:

```ts
import { definePlugin } from "@tynd/plugin-sdk"

let count = 0

export default definePlugin({
  name: "counter",
  methods: {
    increment() { count += 1; return count },
    reset() { count = 0 },
    get() { return count },
  },
})
```

**Native plugin** — no `backend.ts`: the logic lives in Rust (Step 4) and the
frontend entry is a typed client over the generated catalog:

```ts
// client.ts
import { defineTypedPlugin } from "@tynd/plugin-sdk"
import type { CommandsCatalog } from "./greeter.gen"

export const greeter = defineTypedPlugin<CommandsCatalog>("greeter")
export default greeter
```

Calls go through `__tynd__.os_call("plugin", "greeter:<method>")`, so they run
from the webview only; the Bun backend cannot reach native plugin crates.

### Step 3. Define the client (pure JS)

```ts
// client.ts
import { createPluginClient } from "@tynd/plugin-sdk"
import type Plugin from "./backend"

export const greeter = createPluginClient<typeof Plugin>("greeter")
```

Types flow automatically — `greeter.greet({ name: "Alice" })` is a typed `Promise<string>`.

### Step 4. (Native only) Implement the Rust side

```rust
// native/src/lib.rs
use serde_json::Value;
use tynd_plugin_sdk::{register_plugin, unknown_method, ArgsExt, PluginCx, PluginResult, TyndPlugin};

#[derive(Default)]
pub struct GreeterPlugin;

impl TyndPlugin for GreeterPlugin {
    fn name(&self) -> &'static str { "greeter" }

    fn dispatch(&self, _cx: &PluginCx, method: &str, args: Value) -> PluginResult<Value> {
        match method {
            "greet" => {
                let name: String = args.require("name")?;
                Ok(Value::String(format!("Hello, {name}!")))
            }
            _ => Err(unknown_method("greeter", method)),
        }
    }
}

register_plugin!(GreeterPlugin);
```

#### Type generation (Rust → TypeScript)

Native plugins emit a `*.gen.ts` file with the wire-shapes of every IPC
arg/result. Source-of-truth lives in Rust under a `pub mod commands`
annotated with `#[tynd_plugin::commands]`:

```rust
use tynd_plugin_sdk::tynd_plugin;

#[tynd_plugin::commands]
pub mod commands {
    /// Args for greet.
    pub struct GreetArgs {
        /// User name to greet.
        pub name: String,
    }

    /// Internal — not exported.
    #[tynd_plugin::skip]
    pub struct InternalState { pub counter: u32 }
}
```

The macro auto-derives `Serialize` / `Deserialize` / `specta::Type`,
forwards `///` doc comments to TS, rewrites `u64` / `i64` fields to
land as `number` rather than `bigint`, and emits a
`__tynd_export_command_types_<modname>() -> String` helper (the source
module name is suffixed onto the symbol so a single crate can host
multiple `#[commands]`-annotated modules without colliding at link
time; distinct from `#[plugin_impl]`'s `__tynd_export_types` so the
two macros can coexist in one crate).

For a `#[commands]`-only crate, call the helper directly from a
`gen-types` bin (the suffix matches the module name — `pub mod commands`
emits `__tynd_export_command_types_commands`):

```rust
// native/src/bin/gen-types.rs
fn main() {
    print!("{}", plugin_my_plugin::__tynd_export_command_types_commands());
}
```

If the same crate also uses `#[plugin_impl]`, the
`gen_types_bin!(plugin_my_plugin)` macro emits a `main` that calls
the `__tynd_export_types` produced by `#[plugin_impl]` (which already
includes the `#[commands]` types). Pick one:

- `#[plugin_impl]` users → `tynd_plugin_sdk::tynd_plugin::gen_types_bin!(plugin_my_plugin);`
- `#[commands]`-only users → manual `print!("{}", crate::__tynd_export_command_types_<modname>());`

```bash
tynd plugin types   # regenerate <name>.gen.ts of the local native plugins
# or directly:
cargo run -p plugin-my-plugin --bin gen-types --quiet > my-plugin.gen.ts
```

See `crates/plugin-macros/README.md` for the full attribute
reference.

### Step 5. Use it in your app

```ts
// In your app's tynd.config.ts
import type { TyndConfig } from "@tynd/cli"

export default {
  plugins: ["@my-scope/plugin-greeter"],
} satisfies TyndConfig
```

```tsx
// In your frontend: the package root and "/client" both export client.ts
import { greeter } from "@my-scope/plugin-greeter/client"

const msg = await greeter.greet({ name: "Alice" })
console.log(msg)  // "Hello, Alice!"
```

```bash
tynd dev   # plugin compiled into the app's host
```

---

## JavaScript API reference

### `definePlugin(definition)`

Register a pure-JS plugin. Returns a `Plugin<T>` value the host loads.

```ts
import { definePlugin } from "@tynd/plugin-sdk"

export default definePlugin({
  name: "my-plugin",
  methods: {
    method1(arg: string) { /* ... */ },
    async *method2() { yield 1; yield 2 },   // streaming RPC supported
  },
})
```

### `createPluginClient<typeof Backend>(name)`

Build a typed proxy for the frontend. Types are inferred from `typeof Plugin`.

- Streaming generators (`async function*`) surface as awaitable + async-iterable + cancellable handles.
- Regular functions surface as `Promise<Awaited<R>>`.

```ts
import { createPluginClient } from "@tynd/plugin-sdk"
import type Plugin from "./backend"

export const greeter = createPluginClient<typeof Plugin>("greeter")
```

### `onHostEvent(event, handler)`

Subscribe to events your plugin's Rust side emits via `cx.emit`. The host delivers them on the OS event bus, so this listens through `osOn` from `@tynd/core/client/bridge`, under the exact name the plugin emitted (no extra prefix).

```ts
import { onHostEvent } from "@tynd/plugin-sdk"

const off = onHostEvent("greeter:ping", (payload) => {
  console.log("pinged:", payload)
})

// later: off()  to unsubscribe
```

Returns a no-op when called outside a Tynd runtime (safe in unit tests).

### `validate(schema, args)` + `PluginArgsError`

Validate args against any [Standard Schema v1](https://standardschema.dev) validator (valibot, zod, arktype) **before** the IPC round-trip.

```ts
import * as v from "valibot"
import { composePluginClient, defineTypedPlugin, validate } from "@tynd/plugin-sdk"
import type { CommandsCatalog } from "./greeter.gen"

const GreetArgs = v.object({ name: v.pipe(v.string(), v.minLength(1)) })
const api = defineTypedPlugin<CommandsCatalog>("greeter")

export const greeter = composePluginClient(api, {
  greet(args: v.InferInput<typeof GreetArgs>): Promise<string> {
    return api.greet(validate(GreetArgs, args))
  },
})
```

Throws `PluginArgsError` listing every issue on failure. Async schemas are rejected — plugin args must validate synchronously.

### `defineTypedPlugin<C>(name)`

The `client.ts` entry of a native plugin. Returns one object that is
**both** the registration descriptor (loaded from `tynd.config.ts::plugins`)
and a typed RPC proxy (so `await sql.open({ path: ":memory:" })` works). The catalog type comes from the
plugin's auto-generated `*.gen.ts`. Backend-side methods are defined
exclusively in Rust; the JS side ships zero method bodies.

```ts
// plugins/sql/client.ts
import { defineTypedPlugin } from "@tynd/plugin-sdk"
import type { CommandsCatalog } from "./sql.gen"

export const sql = defineTypedPlugin<CommandsCatalog>("sql")
export default sql

// userland
import { sql } from "@tynd/plugin-sql"
app.start()
const id = await sql.open({ path: ":memory:" })
```

### `composePluginClient(client, overrides)`

Compose ergonomic wrappers with a generated or dynamic plugin client. Overrides
remain real own properties; every non-overridden method delegates to the RPC
proxy. For native clients, the registration descriptor is preserved too. Use
this instead of `Object.assign`, which cannot copy methods created lazily by a
proxy.

```ts
import { composePluginClient, defineTypedPlugin } from "@tynd/plugin-sdk"
import type { CommandsCatalog } from "./screen-rec.gen"

const api = defineTypedPlugin<CommandsCatalog>("screen-rec")
export const screenRec = composePluginClient(api, {
  async start(output: string) {
    return api.start({ opts: { output } })
  },
})
```

### `defineTyndPlugin({ name, methods?, hooks? })` — `/hooks` subpath

Lifecycle-aware plugin entry. Combines IPC method registration with a
typed, awaitable hook system built on
[`hookable`](https://github.com/unjs/hookable). Reach for it when the plugin
needs to react to host events instead of just answering IPC calls. Pure IPC
plugins keep using `definePlugin` / `defineTypedPlugin`.

```ts
import { defineTyndPlugin } from "@tynd/plugin-sdk/hooks"
import { getCallContext } from "@tynd/core"

export default defineTyndPlugin({
  name: "metrics",
  methods: {
    async ping() { return "pong" },
  },
  hooks: {
    "app:ready": (cx) => cx.log.info("metrics plugin armed"),
    "ipc:before": (cx, cmd, args) => {
      cx.log.debug(`${getCallContext()?.window} -> ${cmd}`, args)
    },
  },
})
```

Hooks that fire:

| Hook | When | Behavior |
|---|---|---|
| `app:beforeStart(cx, config)` | Before the config frame is sent to the host | Awaited |
| `app:ready(cx)` | When the primary page is ready | |
| `app:beforeQuit(cx)` | On orderly shutdown, before the app's `onClose` handlers | Awaited, capped at 3 s |
| `ipc:before(cx, cmd, args)` | Before every backend call (user functions and plugin methods) | A throw rejects the call. `cmd` is the raw wire name; plugin methods are `$plugin$<name>$<method>` |
| `ipc:after(cx, cmd, result)` | After every successful call | Errors are logged and never change the result |
| `window:created(cx, { label })`, `window:closed(cx, { label })` | A window is created or destroyed | Fired when the host sends `tynd:window` frames |

`cx` is `{ name, log: { debug, info, warn, error }, state }`. A failing hook is
logged and does not stop the other plugins, except `ipc:before` which rejects
the call. `getCallContext()` from `@tynd/core` works inside `ipc:*` hooks.
An unknown hook name throws when the plugin is defined.

### Caller identity and middleware (`@tynd/core`)

Plugin methods run in the same scope as app functions, so they can identify
the calling webview and be wrapped by app-level middleware:

```ts
import { getCallContext } from "@tynd/core"

// inside a plugin method
const caller = getCallContext() // { window, webview, origin: string | null } | undefined
```

`app.use(async (ctx, next) => next())` registers middleware around every backend
function and JS plugin method, with `ctx = { fn, args, caller, plugin? }`
(`plugin` is set for plugin methods, and `fn` is then the method name). The
first registered middleware is the outermost; plugin `ipc:before` / `ipc:after`
hooks run outside all of them. A throw rejects the call and `ctx.args` may be
replaced before `next()`. Middleware is the application's choice, not the
plugin's: a plugin that needs to gate its own methods should check
`getCallContext()` in the method or in an `ipc:before` hook.

### `createHandleEvents(filterId, label?)`

Helper for plugin clients that return a handle with id-bound event
listeners (terminal sessions, sql connections, shortcut handles, …).
Tracks subscriptions created via `on()` and exposes `disposeAll()` to
detach them in one shot. Filters every event by matching the given
field on the payload, so multiple handles can share an event bus
without cross-talk.

```ts
import { createHandleEvents } from "@tynd/plugin-sdk"

const events = createHandleEvents(handle.id, "terminal")
const offData = events.on<{ data: string }>(
  "terminal:data",
  "id",
  (p) => onData(p.data),
)
handle.kill = async () => {
  events.disposeAll()
  await api.kill({ id: handle.id })
}
```

### Type exports

| Type | Purpose |
|---|---|
| `Plugin<T>` | Shape of a plugin definition |
| `PluginDefinition<T>` | Args to `definePlugin` |
| `PluginMethod` | Shape of a single method |
| `PluginClient<P>` | Inferred frontend proxy type |
| `CommandCatalog` | Shape of the `CommandsCatalog` emitted into `*.gen.ts` |
| `TypedClient<C>` | Typed client returned by `defineTypedPlugin<C>` |

---

## Rust API reference

### `TyndPlugin` trait

```rust
pub trait TyndPlugin: Send + Sync + 'static {
    fn name(&self) -> &'static str;
    fn command_names(&self) -> &'static [&'static str] { &[] }
    fn init(&self, cx: &PluginCx) -> PluginResult<()> { Ok(()) }
    fn dispatch(&self, cx: &PluginCx, method: &str, args: Value) -> PluginResult<Value>;
    fn dispatch_rkyv(
        &self,
        cx: &PluginCx,
        method: &str,
        args: &[u8],
    ) -> Option<PluginResult<Vec<u8>>> { None }
    fn on_event(&self, cx: &PluginCx, event: &HostEvent) {}
    fn shutdown(&self, cx: &PluginCx) -> PluginResult<()> { Ok(()) }
}
```

- `name()` must match the JS `definePlugin({ name })` and use canonical
  lowercase kebab-case (`^[a-z][a-z0-9-]{0,30}$`). Registration rejects
  aliases, surrounding whitespace, uppercase names, and reserved names.
- `command_names()` declares stable wire commands used to generate named
  renderer permissions. `#[plugin_impl]` fills it automatically; hand-written
  plugins may override it. Empty means no generated manifest.
- `init()` is called once at startup. Spawn background threads, open files, etc.
- `dispatch()` handles each IPC call. Return `Err(_)` to surface to the frontend's `Promise.reject`.
- `dispatch_rkyv()` is the optional zero-copy fast-path — return `None`
  to fall back to JSON, `Some(Ok(bytes))` / `Some(Err(_))` to handle
  the call. Wired automatically by `#[plugin_command(zero_copy)]`.
- `on_event()` receives host lifecycle events (see
  [`HostEvent`](#hostevent--tyndpluginon_event)). Default no-op.
- `shutdown()` runs once on host shutdown. Plugins flush DBs, close
  sockets, cancel watchers here.

> **`Drop` never runs at exit.** The host keeps every plugin in a static
> registry and never drops it, so `Drop` for the plugin struct or anything it
> owns (child processes, threads, native handles, files needing a flush) does
> not run when the app exits. Put all teardown in `shutdown()`: signal workers,
> drain them with a bounded wait (`GatedStreams::shutdown`,
> `join_with_timeout`, `BoundedPermitPool::wait_idle`) and release OS resources
> explicitly. `shutdown()` blocks the exit path, so keep it bounded.

### `HostEvent` / `TyndPlugin::on_event`

```rust
fn on_event(&self, cx: &PluginCx, event: &HostEvent) {
    match event {
        HostEvent::AppReady => { /* primary webview is interactive */ }
        HostEvent::WindowCreated { label } => { /* ... */ }
        HostEvent::WindowFocus { label, focused } => { /* ... */ }
        _ => {} // the enum is #[non_exhaustive]
    }
}
```

Variants: `AppReady`, `AppExit` (before any plugin `shutdown`),
`WindowCreated { label }`, `WindowClosed { label }`,
`WindowFocus { label, focused }`, `Navigation { window, webview, url }`.
`on_event` runs on a host thread, often the UI thread: return quickly and hand
slow work to a background thread. A panic is caught and logged. Always keep a
`_ => {}` arm. The `Navigation` variant exists but the current host does not
broadcast it yet.

### Caller identity: `PluginCx::caller()`

```rust
fn dispatch(&self, cx: &PluginCx, method: &str, args: Value) -> PluginResult<Value> {
    if let Some(caller) = cx.caller() {
        // caller.window, caller.webview, caller.origin: Option<String>
    }
    Ok(Value::Null)
}
```

`Some(CallerInfo { window, webview, origin })` only while `dispatch` runs on the
thread that received a renderer call. It is `None` for calls from the backend,
`init`, `on_event`, `shutdown` and any thread the plugin spawned (it is not
inherited). Copy the value before handing work to another thread.

### Init scripts and custom URI schemes

Call these from `init`. They apply to webviews created afterwards.

```rust
fn init(&self, cx: &PluginCx) -> PluginResult<()> {
    cx.register_init_script("window.__myPlugin = true;")?;
    cx.register_uri_scheme("myplugin", Arc::new(|req: UriRequest| UriResponse {
        status: 200,
        headers: vec![("Content-Type".into(), "text/plain".into())],
        body: format!("hello {}", req.url).into_bytes(),
    }))?;
    Ok(())
}
```

- Init scripts run before page scripts, after the Tynd shim, in registration
  order, in webviews that show app content (the bundled `tynd://` pages or a
  loopback dev server), including their same-origin frames. They are not
  installed in webviews opened on a remote URL, and they do not run in a
  document whose own location is remote, even after a later navigation. The
  script is wrapped in a block, so top-level `let`/`const` stay local to it
  (use `window.x = ...` or `var` to share state). Blank scripts and scripts over
  256 KiB are rejected. `register_init_script_remote` also runs the script in
  remote-content webviews: such a script must embed no secret, because the
  page can read anything it exposes.
- Scheme names must be lowercase `[a-z0-9+.-]` and start with a letter. `http`,
  `https`, `file`, `data`, `blob`, `about`, `javascript`, `ws`, `wss`, `ftp`,
  `mailto` and anything starting with `tynd` are reserved, as is a name another
  plugin already registered. Registration returns an error in those cases.
- A scheme is installed only in webviews that open on app content, and a
  request whose `Origin`/`Referer` or whose webview shows remote content is
  answered with 403 before it reaches the handler. `register_uri_scheme_remote`
  lifts that restriction for plugins that must serve remote pages.
- `UriRequest` carries `webview` (label of the issuing webview) and `origin`
  (`None` for bundled content, otherwise the page origin such as
  `https://example.com`). A handler registered with `_remote` must check
  `origin` itself.
- A scheme handler runs on the webview's request path: keep it fast and treat
  the request (URL, headers, body) as untrusted. A panic becomes an error
  response.
- Registering after a webview exists has no effect on that webview.

### Host-side `before_dispatch` hook

An app-specific host (custom `main.rs`) can audit or reject every native plugin
call with `tynd_host::plugins::add_before_dispatch`:

```rust
tynd_host::plugins::add_before_dispatch(std::sync::Arc::new(|info| {
    // info.plugin, info.method, info.args, info.caller: Option<CallerInfo>
    if info.plugin == "terminal" && info.caller.as_ref().is_some_and(|c| c.origin.is_some()) {
        return Err("terminal is local-only".into());
    }
    Ok(())
}));
```

Hooks run in registration order before the plugin is dispatched. `Err(reason)`
rejects the call (later hooks and the plugin are skipped); a panicking hook
rejects it too. The chain is empty by default. This is host code, not plugin
code: plugins cannot register it.

### `TyndPlugin::dispatch_rkyv` — zero-copy fast-path

Opt into rkyv-encoded args/results for hot, typed commands where JSON
serde is the bottleneck. The host probes this method first; a `None`
return falls back to the JSON `dispatch` arm. Use the SDK's
[`rkyv_decode_args`](#rkyv_decode_args--rkyv_encode_result) /
[`rkyv_encode_result`](#rkyv_decode_args--rkyv_encode_result) helpers
to keep error formatting uniform.

```rust
fn dispatch_rkyv(
    &self,
    _cx: &PluginCx,
    method: &str,
    args: &[u8],
) -> Option<PluginResult<Vec<u8>>> {
    if method != "process" { return None }
    let parsed: ProcessArgs = match rkyv_decode_args(args) {
        Ok(v) => v,
        Err(e) => return Some(Err(e)),
    };
    Some(rkyv_encode_result(&self.process(parsed)))
}
```

### `TyndPlugin::shutdown` — graceful teardown

Called once by the host on shutdown so plugins flush state cleanly.
Default no-op; override when the plugin owns sockets, DB handles,
watchers, or background threads not stored as `BackgroundHandle`.

```rust
fn shutdown(&self, _cx: &PluginCx) -> PluginResult<()> {
    self.db.lock().flush()?;
    self.watcher.stop();
    Ok(())
}
```

### `register_plugin!(constructor)`

Expose your plugin to the host-generated `main.rs`.

```rust
register_plugin!(MyPlugin::default());
// or for stateful plugins:
register_plugin!(MyPlugin { sessions: DashMap::new() });
```

### `Registry<T>`

Concurrent handle table for plugin-side state. Centralises the
`DashMap<u64, Mutex<T>>` + atomic next-id + capacity-cap pattern so
every plugin doesn't roll its own. Each `insert` is admission-checked
against a hard cap (atomic counter, race-free) and returns a
freshly-minted `u64` id the frontend uses to address that handle on
later calls. `remove` frees the slot for reuse.

```rust
use tynd_plugin_sdk::Registry;

const MAX_CONNECTIONS: usize = 256;

let connections: Registry<Connection> = Registry::new(MAX_CONNECTIONS);
let id = connections.insert(conn)?;             // Err when at capacity
let result = connections.with(id, |c| c.query("SELECT 1"))?;
connections.remove(id);
```

Entries are individually locked: `with(id, f)` runs `f` while holding that
entry's mutex (the table itself is not locked), so keep `f` short. For long
work, store `Arc<M>` values (`Registry<Arc<M>>`) and use `get_arc(id)` to clone
the `Arc` out and run lock-free. Also available:

- `with_arg(id, key, operation, f)`: a missing id is reported as
  `InvalidArg { key, .. }`, with no `contains()` + `with()` race.
  `get_arc_arg(id, key, operation)` is the `Arc` equivalent.
- `snapshot(f) -> Vec<R>`: run `f` on every live entry; entries removed in
  between, or whose `f` errors, are skipped.

Canonical usage in the plugin tree:

- [`plugins/standalone/audio/native/src/lib.rs`](https://github.com/kvnpetit/tynd/tree/main/plugins/standalone/audio/native/src/lib.rs) — process-owned audio stream registry

### `PluginCx`

Context passed to `init()` and `dispatch()`. Lets your plugin emit events, call core OS APIs, and spawn background workers.

```rust
impl PluginCx {
    pub fn emit(&self, event: &str, payload: Value);
    pub fn emitter(&self) -> EmitFn;          // clone for use in spawned threads
    pub fn os_call(&self, api: &str, method: &str, args: Value) -> PluginResult<Value>;
    pub fn caller(&self) -> Option<CallerInfo>;
    pub fn register_init_script(&self, script: impl Into<String>) -> PluginResult<()>;
    pub fn register_uri_scheme(&self, name: impl Into<String>, handler: UriHandler) -> PluginResult<()>;
    pub fn register_init_script_remote(&self, script: impl Into<String>) -> PluginResult<()>;
    pub fn register_uri_scheme_remote(&self, name: impl Into<String>, handler: UriHandler) -> PluginResult<()>;
    pub fn with_allowed_permissions<I, S>(self, permissions: I) -> Self;
    pub fn spawn_interval<F>(&self, name: impl Into<String>, interval: Duration, body: F) -> PluginResult<BackgroundHandle>
    where F: FnMut() + Send + 'static;
}
```

#### `PluginCx::with_allowed_permissions(permissions)` — capability gating

Restrict context to exact core OS commands. Host wires this from
`plugins: [{ pkg, permissions: [...] }]` in `tynd.config.ts`; any `os_call`
outside list returns `PluginError::NotPermitted`. Selectors use
`api:method`; `api:*` grants every method on one API. API names are normalized
so `"FS"` and `" fs "` resolve identically. Method names remain case-sensitive,
matching host dispatch.

Three states matter:

- **`Some(empty)` (default)** — deny-all. A newly constructed context cannot
  reach a host command until permissions are attached.
- **`Some(set)`** — gated; only exact selectors or explicit wildcards pass. An
  explicit set is the normal host-wired production state.
- **`None`** — unrestricted, available only through the explicit
  `with_allowed_apis_unrestricted()` builder for controlled embeddings and
  tests. Generated Tynd hosts do not use this state.

```rust
let cx = PluginCx::new(emit)
    .with_dispatch(host_dispatch)
    .with_allowed_permissions(["fs:readText", "http:*"]);
```

Generated hosts never use unrestricted
mode. Read-only security checks are granted as exact methods; future security
commands do not inherit access automatically.

### `ArgsExt` — extract typed args

Cleaner than `args.get("k").and_then(Value::as_str)` chains.

```rust
use tynd_plugin_sdk::ArgsExt;

fn open(args: Value) -> PluginResult<()> {
    let path: String = args.require("path")?;          // Err if missing
    let timeout: u64 = args.opt("timeout").unwrap_or(30);   // None if absent or wrong type
    Ok(())
}
```

### `BackgroundHandle`

Returned by `cx.spawn_interval`. `Drop` automatically signals exit + joins the thread, so storing one in plugin state is enough for clean shutdown.

```rust
let handle = cx.spawn_interval("poll", Duration::from_secs(5), || { /* ... */ })?;
// later (or on Drop): handle.stop(); handle.join();
```

### `BoundedPermitPool` — bound concurrent native work

A fixed-capacity pool; permits return on drop, including on panic.

```rust
let pool = BoundedPermitPool::per_core("decode"); // or ::new(4, "decode")
let _permit = pool.acquire_timeout("decode", Duration::from_secs(15))?;
```

| Method | Behavior |
|---|---|
| `try_acquire(op)` | Never blocks. `Ok(None)` when every permit is held |
| `acquire_timeout(op, d)` | Waits up to `d`, then fails with `"{op}: {name} busy (waited Ns) - retry later"` |
| `acquire(op)` | Blocks without bound. A command runs on a shared call-pool worker, so parking it here starves unrelated plugin calls: avoid it in commands |
| `wait_idle(d)` | `true` once every permit is back; use it to drain work in `shutdown()` |
| `per_core(name)` | Capacity = available parallelism (at least 1) |

### `GatedStreams` — reserve, start, chunks, end

Helper for long-running streaming commands: bounded, cancellable, with a start
gate so no chunk is emitted before the client listens.

1. The command calls `spawn_stream(...)`: it takes a permit (timed), registers a
   cancel flag and a start gate, spawns a worker parked on the gate and returns
   the stream id.
2. The client subscribes to the plugin's events, then calls a `startStream`
   command that calls `start(op, id)`.
3. The worker emits chunk events, then exactly one end event
   `{ stream, ok, error? }`.

```rust
let streams = Arc::new(GatedStreams::new(4, "llm", GatedLimits::default()));
let id = streams.spawn_stream("generate", "llm-gen", cx.emitter(), "llm:end", move |id, cancel| {
    while !cancel.load(Ordering::Acquire) { /* emit chunks */ }
    Ok(())
})?;
```

Guarantees: a busy pool fails with a "busy" error instead of blocking; a stream
whose `start` never arrives (page reload) expires after `start_wait` (default
30 s), frees its permit and emits `end { ok: false }`; a worker error or panic
becomes `end { ok: false, error }`; `cancel(id)` works before and after start;
`run_sync(op, work)` runs short work on the calling thread inside a permit.
Call `shutdown(deadline)` (suggested `DEFAULT_SHUTDOWN_DEADLINE`, 5 s) from your
plugin's `shutdown()`: it cancels everything, refuses new work and waits up to
the deadline; it returns `false` if native work is still running.

### `join_with_timeout(handle, timeout)`

Bounded `JoinHandle` join that never blocks past `timeout` and spawns no helper
thread. Returns `JoinOutcome::Finished(v)`, `Panicked`, or `TimedOut(handle)`
(the handle comes back so you can retry or detach). Use it in `shutdown()`
instead of a busy-poll loop.

### Plugin proc-macros — `#[plugin]` / `#[plugin_impl]` / `#[command]`

Skip the hand-written `TyndPlugin` impl. Annotate a struct with
`#[tynd_plugin::plugin(name = "…")]` and the impl block with
`#[tynd_plugin::plugin_impl]`, mark each command method `#[command]`,
and the SDK generates the dispatch arms, per-command `<Method>Args`
structs, `<Method>Result` aliases, the specta TS exporter, and the command list
used by the host's named permission manifest.

```rust
use tynd_plugin_sdk::{tynd_plugin, PluginCx, PluginResult};

#[tynd_plugin::plugin(name = "greeter")]
#[derive(Default)]
pub struct GreeterPlugin;

#[tynd_plugin::plugin_impl]
impl GreeterPlugin {
    /// Say hello.
    #[command]
    pub fn greet(&self, _cx: &PluginCx, name: String) -> PluginResult<String> {
        Ok(format!("Hello, {name}!"))
    }

    // Reserved lifecycle hook: not exposed to the renderer.
    pub fn plugin_shutdown(&self, _cx: &PluginCx) -> PluginResult<()> {
        Ok(())
    }
}
```

The macro accepts an optional second `cx: &PluginCx` arg before the
real params. It is recognised either by name (`cx` / `ctx`) or by type
(any `&PluginCx` reference, regardless of binding name — including the
leading-underscore `_cx: &PluginCx` idiom for "I won't read this");
plain `_cx` without the type annotation isn't matched and falls through
to a regular field. The arg is threaded into the dispatch arm only when
present. `async fn` command bodies work too — the generated arm wraps
them in
[`__internal::block_on`](https://github.com/kvnpetit/tynd/blob/main/crates/plugin-sdk/src/lib.rs) (no tokio runtime; see the
WARNING in that doc).

Typed plugins may also define the reserved, untagged methods
`plugin_init(&self, &PluginCx) -> PluginResult<()>` and
`plugin_shutdown(&self, &PluginCx) -> PluginResult<()>`. The macro forwards
them to `TyndPlugin::init` and `TyndPlugin::shutdown`; tagging either with
`#[command]` is rejected so lifecycle operations never become renderer RPCs.

Each generated wire command gets renderer capability permissions:

- `<plugin>:allow-<command>` and `<plugin>:deny-<command>`
- `<plugin>:allow-all` and `<plugin>:deny-all` permission sets
- `<plugin>:default`, an alias for `allow-all`

For example, `start_capture` becomes the wire command `startCapture` and the
named permission `audio:allow-start-capture`. These permissions gate
renderer → plugin calls. They are separate from
`plugins: [{ permissions: ["fs:readText"] }]`, which gates plugin → host calls.
Internally, renderer grants use a dedicated plugin namespace, so a plugin named
`fs` cannot inherit or grant the core `fs:*` API.

The macro emits paths into `::tynd_plugin_sdk::specta::*` /
`::tynd_plugin_sdk::specta_typescript::*` / `::tynd_plugin_sdk::specta_serde::*`
— the SDK re-exports these crates so plugin authors never need to add
`specta`, `specta-typescript`, or `specta-serde` to their own
`[dependencies]`. The `specta_serde::PhasesFormat` exporter format is required for
`#[serde(rename_all = "camelCase")]` to actually rename fields in the
generated `.gen.ts` (specta-typescript on its own prints raw Rust
idents).

#### `#[plugin_command(zero_copy)]` — opt into rkyv

Tag a `pub struct` inside `#[tynd_plugin::commands]` mod with
`#[plugin_command(zero_copy)]` and the macro auto-derives the rkyv
`Archive` / `Serialize` / `Deserialize` trio alongside serde + specta.
The host then prefers the `dispatch_rkyv` path for that command,
skipping the JSON round-trip.

```rust
#[tynd_plugin::commands]
pub mod commands {
    use tynd_plugin_sdk::tynd_plugin;

    #[tynd_plugin::plugin_command(zero_copy)]
    pub struct ProcessArgs {
        pub n: u32,
        pub payload: String,
    }
}
```

> **rkyv re-export is public surface.** `tynd_plugin_sdk::rkyv` is
> intentionally NOT `#[doc(hidden)]` (unlike `serde` / `specta` /
> `specta-typescript` / `specta-serde`, which are macro-internal escape
> hatches). Plugin authors writing zero-copy types touch `rkyv::Archive`
> / `rkyv::Serialize` / `rkyv::Deserialize` directly in their own
> signatures, and the rkyv archive format is what flows across the
> dispatcher boundary. A breaking rkyv bump (e.g. 0.8 → 0.9) therefore
> counts as a breaking SDK bump and ships in a major-version SDK
> release. Stay on the SDK-pinned rkyv version; do not add `rkyv` to
> your own `[dependencies]` with a different pin.

### `rkyv_decode_args` / `rkyv_encode_result`

Helpers for plugins implementing the `dispatch_rkyv` fast-path
manually. Wrap the rkyv API with consistent `"rkyv decode:"` /
`"rkyv encode:"` error prefixes so plugin logs grep cleanly.

```rust
use tynd_plugin_sdk::{rkyv_decode_args, rkyv_encode_result};

let parsed: ProcessArgs = rkyv_decode_args(args)?;
let bytes: Vec<u8> = rkyv_encode_result(&self.process(parsed))?;
```

### `unknown_method(plugin, method)` helper

Canonical fallthrough for `dispatch`:

```rust
match method {
    "ping" => Ok(Value::String("pong".into())),
    _ => Err(unknown_method("my-plugin", method)),
}
```

### Error types

`PluginError` is a `#[derive(thiserror::Error)] enum` carrying the
canonical failure shapes plugins surface to the host. Plugins can also
use the `format!(...).into()` idiom, which lands in the `Custom` variant.

```rust
#[derive(Debug, thiserror::Error)]
pub enum PluginError {
    /// A required arg was absent on the incoming JSON.
    #[error("missing required arg '{key}'")]
    MissingArg { key: String },

    /// An arg was present but failed to deserialize into the requested type.
    #[error("invalid arg '{key}': {detail}")]
    InvalidArg { key: String, detail: String },

    /// Whole `args` blob failed to deserialize into a method's generated `Args` struct.
    #[error("invalid args for method '{method}': {detail}")]
    InvalidArgsBlob { method: String, detail: String },

    /// Plugin tried to call an OS API outside its declared `apis: [...]` allow-list.
    #[error("plugin capability: not permitted to call {api}.{method} (declared apis: {allowed:?})")]
    NotPermitted { api: String, method: String, allowed: Vec<String> },

    /// `cx.os_call(...)` was invoked from a context with no host dispatcher.
    #[error("plugin capability: host dispatch not wired")]
    DispatchNotWired,

    /// `dispatch` fallthrough — see [`unknown_method`].
    #[error("{plugin}.{method}: unknown method")]
    UnknownMethod { plugin: String, method: String },

    /// Free-form error — `format!(...).into()` lands here.
    #[error("{0}")]
    Custom(String),

    /// `std::io::Error` bubbled up via `?`.
    #[error(transparent)]
    Io(#[from] std::io::Error),

    /// `serde_json::Error` bubbled up via `?`.
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

pub type PluginResult<T> = Result<T, PluginError>;
```

`From<String>` / `From<&str>` / `From<Box<dyn Error + Send + Sync>>` map
ad-hoc errors into the `Custom` variant, so plugins keep using the
`format!(...).into()` idiom.

---

## Testing

The SDK ships test utilities so plugins don't need a running host to unit-test their dispatch logic.

### JS plugins — call methods directly

The `definePlugin` descriptor exported by `backend.ts` exposes the methods on `descriptor.methods`. Tests invoke them in Bun with no host bridge — no `__tynd__` mock required.

```ts
import { expect, test } from "bun:test";
import counter from "./backend.ts";

test("increment counts", async () => {
  expect(await counter.methods.increment()).toBe(1);
});
```

Run with `bun test`.

### `PluginCx::test()` — quick context

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tynd_plugin_sdk::PluginCx;

    #[test]
    fn ping_returns_pong() {
        let cx = PluginCx::test();
        let p = MyPlugin::default();
        let out = p.dispatch(&cx, "ping", json!(null)).unwrap();
        assert_eq!(out, json!("pong"));
    }
}
```

### `test_cx()` — capture emitted events

```rust
use tynd_plugin_sdk::test_cx;

#[test]
fn emits_event_on_call() {
    let (cx, events) = test_cx();
    let p = MyPlugin::default();
    p.dispatch(&cx, "broadcast", json!({"msg": "hi"})).unwrap();
    let recorded = events.events();
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].0, "my-plugin:broadcast");
}
```

### Mock `os_call`

```rust
use std::sync::Arc;
use serde_json::{json, Value};

let dispatch = Arc::new(|api: &str, method: &str, _args: Value| {
    Ok(json!(format!("{api}.{method}")))
});
let cx = PluginCx::test().with_dispatch(dispatch);
```

---

## package.json layout

A native plugin's `package.json` declares the Rust crate path and its Cargo package name (`crateName`; the CLI maps `-` to `_` for the Rust identifier) so the CLI can wire it into the generated workspace:

```json
{
  "name": "@my-scope/plugin-greeter",
  "version": "1.0.0",
  "license": "Apache-2.0",
  "type": "module",
  "exports": {
    ".": "./client.ts",
    "./client": "./client.ts"
  },
  "files": ["client.ts", "greeter.gen.ts", "native/**", "!**/target/**"],
  "dependencies": {
    "@tynd/plugin-sdk": "^1.0.0"
  },
  "peerDependencies": {
    "@tynd/core": "^1.0.0"
  },
  "tynd": {
    "native": {
      "path": "./native",
      "crateName": "my-scope-plugin-greeter"
    }
  }
}
```

Community plugins omit `tynd.kind` (official plugins set `"host"` or `"standalone"`). A pure JS plugin replaces the `tynd.native` block with `"backend": true`, adds a `"./backend": "./backend.ts"` export, and ships `backend.ts` + `client.ts` (no `.gen.ts`, no `native/` folder):

```json
{
  "exports": {
    ".": "./client.ts",
    "./client": "./client.ts",
    "./backend": "./backend.ts"
  },
  "files": ["backend.ts", "client.ts"],
  "tynd": { "backend": true }
}
```

## Examples

Real-world plugins built on this SDK:

- [`@tynd/plugin-sql`](https://github.com/kvnpetit/tynd/tree/main/plugins/standalone/sql) — bundled SQLite, stateful handles
- [`@tynd/plugin-terminal`](https://github.com/kvnpetit/tynd/tree/main/plugins/standalone/terminal) — PTY sessions, `cx.spawn_interval` polling pattern
- [`@tynd/plugin-command-palette`](https://github.com/kvnpetit/tynd/tree/main/plugins/standalone/command-palette) — pure-JS, complex client-side state

Stable host capabilities such as
[`@tynd/plugin-keyring`](https://github.com/kvnpetit/tynd/tree/main/plugins/host/keyring)
are host-backed official plugins, not examples of custom native plugin dispatch.

## Related repos

- [`@tynd/cli`](https://github.com/kvnpetit/tynd/tree/main/packages/cli) — scaffold, dev, build, ship
- [`@tynd/core`](https://github.com/kvnpetit/tynd) (in `kvnpetit/tynd`) — core OS APIs and runtime
- [Standalone official plugins](https://github.com/kvnpetit/tynd/tree/main/plugins/standalone) — sql, terminal, llm, whisper, …
- [`@tynd/mcp`](https://github.com/kvnpetit/tynd/tree/main/packages/mcp) — Model Context Protocol server
- [Documentation site](https://github.com/kvnpetit/tynd/tree/main/apps/docs)

## License

Apache-2.0. See [LICENSE](./LICENSE) and [NOTICE](./NOTICE).
