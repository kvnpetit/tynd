# Tynd plugins — authoring guide

Plugins extend a Tynd app without touching the framework core. Each plugin
is an npm package. Whether it ships native code (Rust) or stays pure JS
depends on what the feature needs.

## Layout

```
plugins/<name>/                     # official plugins live here; community
                                    # plugins are regular npm packages.
├── package.json
├── README.md
├── client.ts                       # frontend API — always present.
├── backend.ts                      # backend handlers — only if the plugin
                                    # registers methods exposed to the
                                    # frontend. Omit for pure-frontend libs.
└── native/                         # Rust sources — only for native plugins.
    ├── Cargo.toml
    └── src/lib.rs
```

Three plugin shapes:

1. **Frontend lib** (`i18n`, `theme`, `qr`, `command-palette`) — `client.ts`
   only. Consumers `bun add` + `import` from `<pkg>/client`. No entry in
   `config.plugins` and no `app.start({ plugins: [...] })`.
2. **JS backend plugin** (`deeplink`, `analytics`) — `backend.ts` declares
   handlers via `definePlugin`, `client.ts` wraps them with
   `createPluginClient`. Consumer adds the default export of `backend.ts`
   to `app.start({ plugins: [...] })`.
3. **Native plugin** (`sql`, `terminal`, `updater`, `keyring`, `autolaunch`, `clipboard-history`) — everything
   above **plus** a `native/` Rust crate implementing `TyndPlugin`. Requires
   `runtime: "full-custom"` or `"lite-custom"` on the consuming app; the
   CLI links the Rust crate at build time.

## Versioning policy

Plugins track framework compatibility via `peerDependencies`. Use caret
ranges so minor patches on the framework don't force a plugin re-release,
while major bumps require explicit opt-in.

### Pre-1.0 (current phase)

```json
{
  "peerDependencies": {
    "@tynd/core":       "^0.3.0",
    "@tynd/plugin-sdk": "^0.3.0"
  }
}
```

Semver treats `0.x` as unstable — `^0.3.0` resolves to `>=0.3.0 <0.4.0`,
so the plugin works with any `0.3.x` framework but flags a peer-dep
warning against `0.4.0`. Bump the plugin's peer range when the framework
cuts a new minor. Pin the plugin's own version to the framework's
`minor.patch` to make the drift obvious.

### Post-1.0 (when the framework stabilizes)

```json
{
  "peerDependencies": {
    "@tynd/core":       "^1.0.0",
    "@tynd/plugin-sdk": "^1.0.0"
  }
}
```

Now `^1.0.0` spans the whole `1.x` line. Plugins only need a re-release
for major bumps (`2.0.0`). Breaking changes to the plugin SDK or the
frontend client are documented in `CHANGELOG.md` and gated behind major
versions.

### Rust compile-time check

Native plugins also depend on `tynd_plugin_sdk` through Cargo:

```toml
[dependencies]
tynd_plugin_sdk = { workspace = true }
```

The CLI resolves `tynd_plugin_sdk` to the SDK shipped with the current
framework checkout. A mismatch between what the plugin expects (via the
trait signature) and what the framework provides surfaces as a compile
error — never a silent runtime crash.

## Naming

- Official plugins: `@tynd/plugin-<name>` — reserved for the core team.
- Community plugins: any unscoped name works (`awesome-tynd-foo`,
  `@acme/plugin-bar`, etc.). No hard convention.

Set `tynd.native.crateName` in `package.json` to the final Rust crate
name (snake_case). Defaults to a mangled form of the npm name if omitted.

## Consuming a plugin

Frontend lib — just install and import:

```ts
// frontend/app.tsx
import { t, setLocale } from "@tynd/plugin-i18n/client"
```

JS backend plugin — register explicitly:

```ts
// backend/main.ts
import { app } from "@tynd/core"
import deeplink from "@tynd/plugin-deeplink/backend"

app.start({ window: { title: "My App" }, plugins: [deeplink] })
```

Native plugin — same as JS backend plus declare in config:

```ts
// tynd.config.ts
export default {
  runtime: "lite-custom",
  apis: ["clipboard", "store"],
  plugins: ["@tynd/plugin-clipboard-history"],
  backend: "backend/main.ts",
  frontendDir: "dist",
}
```

The CLI picks up the plugin's Rust sources, adds them as path deps in the
generated `.tynd/build/Cargo.toml`, and registers the boxed plugin in the
generated `main.rs` before starting the runtime.

## Capability (native plugins)

A native plugin can declare which core OS APIs it is allowed to reach
through `cx.os_call`. The host enforces the list at runtime — any call
outside the list fails with `"plugin capability: not permitted to call
<api>.<method>"` before the dispatcher runs.

```ts
// tynd.config.ts
export default {
  runtime: "lite-custom",
  apis: ["fs", "http", "store"],
  plugins: [
    "@tynd/plugin-clipboard-history",               // unrestricted (default)
    { pkg: "@acme/plugin-telemetry", apis: ["http"] }, // http-only
    { pkg: "@acme/plugin-vault",     apis: ["fs"]   }, // fs-only
  ],
  backend: "backend/main.ts",
  frontendDir: "dist",
}
```

Inside the plugin crate, calls that should honour the allow-list go
through `cx.os_call(...)` instead of reaching into `tynd_host` directly:

```rust
use tynd_plugin_sdk::{PluginCx, PluginResult, TyndPlugin};

fn dispatch(&self, cx: &PluginCx, method: &str, args: serde_json::Value)
    -> PluginResult<serde_json::Value>
{
    match method {
        "read-config" => cx.os_call("fs", "readText", args),
        "send-metric" => cx.os_call("http", "post", args),
        _ => Err(format!("unknown method: {method}").into()),
    }
}
```

**Scope of enforcement.** The capability gate only guards calls made
through `cx.os_call`. A native plugin still has full Rust access to
`std::fs`, `std::net`, etc. — Tynd doesn't sandbox the process. Declaring
`apis` on an untrusted plugin therefore expresses *intent*, not
isolation; use it with `security.configure({ fs, http })` at the app
level for meaningful restrictions.

**JS-only plugins** (no `native/` directory) don't carry a capability
gate: their backend runs in the user Bun / QuickJS process with the same
access as user code. The `apis` field is silently ignored for them.

## Native plugin SDK helpers

`tynd_plugin_sdk` ships a handful of ergonomics helpers so plugin code
doesn't re-implement the same plumbing.

### `ArgsExt` — typed argument access

Every dispatch method receives a `serde_json::Value`. Pulling fields
out by hand is repetitive and error-prone — use the extension trait:

```rust
use tynd_plugin_sdk::ArgsExt;

fn open(args: &Value) -> PluginResult<Value> {
    let path: String = args.require("path")?;   // errors: "missing required arg 'path'"
    let port: u16    = args.require("port")?;   // errors: "invalid arg 'port': ..."
    let mode: String = args.opt("mode").unwrap_or_else(|| "rw".into());
    let debug: bool  = args.opt("debug").unwrap_or(false);
    Ok(Value::Null)
}
```

Works with any `DeserializeOwned` type — strings, numbers, bools,
custom structs, nested objects.

### `cx.spawn_interval` — periodic background worker

Replaces the hand-rolled `AtomicBool + thread::spawn + sleep loop`
pattern. Returns a `BackgroundHandle` whose `Drop` stops the worker
and joins it — store it in plugin state to get free teardown:

```rust
use std::time::Duration;
use tynd_plugin_sdk::{BackgroundHandle, PluginCx};

struct MyPlugin {
    poll: parking_lot::Mutex<Option<BackgroundHandle>>,
}

fn init(&self, cx: &PluginCx) -> PluginResult<()> {
    let emit = cx.emitter();
    let handle = cx.spawn_interval(
        "my-plugin-tick",
        Duration::from_millis(500),
        move || {
            emit("my-plugin:tick", serde_json::json!({ "now": "..." }));
        },
    );
    *self.poll.lock() = Some(handle);
    Ok(())
}
```

The sleep between ticks is broken into 250 ms slices so `stop()` /
`Drop` takes effect in bounded time even when `interval` is large.

### `test_cx()` — unit-test context

Builds a `PluginCx` whose `emit` captures every event into a
`Vec<(String, Value)>`, so plugin tests can assert side-effects
without wiring a real host:

```rust
use tynd_plugin_sdk::test_cx;

#[test]
fn ping_emits_pong() {
    let (cx, events) = test_cx();
    MyPlugin::default()
        .dispatch(&cx, "ping", serde_json::json!(null))
        .unwrap();
    assert_eq!(events.events()[0].0, "ping:pong");
}
```

### `validate(schema, args)` — client-side arg validation

TS helper that short-circuits bad shapes before they hit the native side.
Works with any Standard Schema v1 validator (valibot, zod, arktype):

```ts
import * as v from "valibot"
import { defineNativePlugin, validate } from "@tynd/plugin-sdk"

const HelloArgs = v.object({ who: v.pipe(v.string(), v.minLength(1)) })

export default defineNativePlugin("foo", (native) => ({
  hello(args: v.InferInput<typeof HelloArgs>) {
    return native<string>("hello", validate(HelloArgs, args))
  },
}))
```

Throws `PluginArgsError` with every issue listed on shape mismatch — no
round-trip to Rust, no generic serde error message.

## CLI commands for plugin authoring

- `tynd plugin-add <pkg>` — one-shot plugin install: runs `bun add <pkg>`
  and edits `tynd.config.ts::plugins` in the same pass. Native plugins
  warn about a runtime mismatch; `--auto-runtime` rewrites `runtime:` to
  the matching `-custom` value. `--skip-install` edits config only (for
  in-workspace plugins).
- `tynd plugin-new <name> [--native]` — scaffold a new plugin tree
  (`backend.ts`, `client.ts`, `package.json`, optional `native/` Rust
  crate). Scope is inferred from the nearest `package.json`; override
  with `--scope @myco`.
- `tynd plugin-check [dir]` — validate a plugin package before publish
  (`package.json` shape, required files, native crate layout,
  `register_plugin!` + `TyndPlugin` impl). Exits 1 on any failure so CI
  can gate on it.
- `tynd plugin-list` — show every plugin the current app declares, with
  kind (`js` / `native` / `bundled` / `missing`) and declared capability
  allow-list. Adds a warning when any plugin package can't be resolved.
- `tynd doctor` — environment health check (Bun version, host binary,
  GitHub reach, managed Rust toolchain).

The build pipeline also runs a **runtime-plugin compatibility pre-flight**
at the top of `tynd dev` / `tynd build`: declaring a native plugin with a
non-custom runtime (`lite` / `full`) fails fast with the exact runtime
swap to make in `tynd.config.ts`.

## Hot reload coverage

`tynd dev` keeps every surface live without manual restart:

| Surface                                 | Behaviour                                                            |
|-----------------------------------------|----------------------------------------------------------------------|
| Frontend (Vite / any dev server)        | HMR owned by the framework                                           |
| Frontend static HTML/CSS/TS/JS          | `reload-frontend\n` on stdin — webview re-fetches from disk          |
| Frontend assets (img/font/video/avif…)  | Same — watcher covers every web MIME, cache bypassed in dev          |
| Backend TS (lite)                       | Rebuild dev bundle + `reload\n` — QuickJS swap, window kept          |
| Backend TS (full)                       | `reload\n` — Bun respawns + re-reads source, window kept             |
| Installed JS plugins                    | Rides the backend watcher (imported by the entry)                    |
| **Workspace-linked JS plugins**         | Dedicated watcher on the plugin source tree — same backend reload    |
| Native plugin `src/**/*.rs`             | Cargo rebuild + atomic host swap — old window stays alive during compile |
| Native plugin `Cargo.toml` / `.lock`    | Same — dep changes trigger rebuild                                   |
| `.env*`                                 | Full restart (fresh process env)                                     |
| `tynd.config.ts` / `package.json`       | Full restart (fresh config)                                          |

Static frontend hot-reload relies on a dev-mode bypass in `tynd-host`'s
custom scheme handler: in `--debug` runs the cache is skipped so editor
saves land on the very next reload. Packed binaries keep the cached
fast path untouched.

**Every open webview** reloads in lockstep — the primary window and
every secondary (Settings / About / helper popups) receive the same
`window.location.reload()` call, so multi-window apps stay in sync
after a hot swap. The dev error overlay is also mirrored to every
webview so a broken backend is visible whichever window is focused.

Events that stack during a reload coalesce to the highest-priority
reason (`config > native > backend > frontend`) so a burst of saves
produces one correct reload instead of a rebuild storm.

The context has no dispatcher by default — if the method under test
calls `cx.os_call(...)`, pair `test_cx()` with `cx.with_dispatch(...)`
to stub the target APIs.
