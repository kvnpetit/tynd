# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [1.0.0] — 2026-04-22

First stable release. The framework ships the runtime, CLI, and every
first-party plugin in lockstep under `1.0.0`; future minor releases stay
on the same SemVer line.

### ⚠ Breaking Changes

- **plugins:** Extract `sql`, `terminal`, `updater`, `keyring`,
  `autolaunch`, `shortcuts`, `log`, `power`, and `clipboard-history` out
  of `@tynd/core` into standalone `@tynd/plugin-*` packages. Preset
  runtimes (`full`, `lite`) still bundle every first-party plugin so the
  user-visible capability is identical; imports move from
  `@tynd/core/client` to `@tynd/plugin-<name>/client`. Custom runtimes
  (`full-custom`, `lite-custom`) pick plugins à la carte via
  `tynd.config.ts::plugins`, including third-party plugins.

### Added

- **plugin-sdk:** publish `@tynd/plugin-sdk` + Rust `tynd_plugin_sdk`
  crate so community plugins can ship JS-only *or* native code with a
  single trait + helper.
- **core:** `onHostEvent` helper in the plugin SDK so clients subscribe
  to backend-emitted events without redeclaring `TyndGlobal`.
- **core:** `clearFrontendListeners()` + auto-cleanup on `app.onClose`.
- **runtime:** `safeStringify()` shared by full + lite handles BigInt
  and circular refs uniformly (BigInt serialises to string; non-JSON
  values surface as explicit errors instead of silent `null`).
- **runtime:** stdin buffer capped at 8 MiB to reject malformed IPC
  streams; stream credit capped at `STREAM_CREDIT` to keep ACK flow
  bounded.
- **store:** `has`, `values`, `entries`, `length`, `reload` round out
  the Map-like surface.
- **plugin-sdk:** `PluginCx::os_call(api, method, args)` lets a native
  plugin reach core OS APIs through the SDK. Opt into the capability
  gate from `tynd.config.ts` with `plugins: [{ pkg, apis: ["fs"] }]` —
  calls outside the allow-list fail with a `"plugin capability: not
  permitted"` error before hitting the dispatcher. JS-only plugins run
  unsandboxed; the field is advisory for them.
- **plugin-sdk:** DX helpers to drop the boilerplate out of a typical
  native plugin: `ArgsExt::require<T>` / `opt<T>` (typed extractor on
  `serde_json::Value`), `PluginCx::spawn_interval(name, interval, body)`
  (periodic background worker with `BackgroundHandle` that stops + joins
  on `Drop`), and `test_cx()` (unit-test context whose `emit` captures
  events into an `EventCapture`).
- **cli:** custom-build binaries carry the user's app name. Generated
  `.tynd/build/Cargo.toml` uses `<appName>` as the `[[bin]]` name; the
  plugin-less path copies `tynd-{full,lite}` to
  `target/release/<appName>` after `cargo build`.
- **cli:** `tynd plugin-new <name> [--js|--native]` scaffolds a
  ready-to-hack plugin package with working stubs + unit tests.
  `--scope <scope>` overrides the default; when omitted the scope is
  inferred from the nearest `package.json`.
- **cli:** `tynd dev` now supports `runtime: "lite-custom"` /
  `"full-custom"` — calls `buildCustomHost()` on start so the dev loop
  works with the exact plugin + API set declared in `tynd.config.ts`.
- **cli:** atomic native host swap during rebuild — `tynd dev`
  compiles the next binary before killing the running one, so cargo's
  multi-second rebuilds no longer leave a dead window on screen.
- **host:** `BackendEvent::Reload` + `BackendEvent::Error` broadcast to
  every open webview, not just the primary. Multi-window apps now
  refresh and surface dev-mode error overlays uniformly.
- **cli:** workspace-linked plugin sources hot-reload — when a declared
  plugin resolves to a path outside `node_modules` (workspace protocol,
  `bun link`, `file:` dep), `tynd dev` installs a dedicated watcher.
- **cli:** extended watcher file-type coverage — native watchers now
  include `Cargo.lock`, frontend watchers cover fonts (woff/ttf/otf),
  video (mp4/webm), audio (mp3/ogg), AVIF, BMP, and TSX entries.
- **host:** dev-mode static asset bypass + `reload-frontend` stdin
  command — `tynd_host::scheme::set_dev_mode(true)` skips the memory
  cache and reads each asset from disk; a new
  `ReloadHandle::reload_frontend_only()` sends `BackendEvent::Reload`
  without swapping the backend.
- **cli:** `tynd dev` hot-reloads static frontends — when no external
  dev server is configured, the CLI watches `cfg.frontendDir` and
  streams `reload-frontend\n` to the host.
- **cli:** `.env*` watchers (`env`, `env.local`, `env.development`,
  `env.development.local`) trigger a full restart.
- **cli:** hot-reload for native plugins — `tynd dev` watches every
  declared native plugin's `src/**/*.rs` + `Cargo.toml`. Edits trigger a
  debounced cargo rebuild + host respawn (window recreated). Coalesce
  rule: `config > native > backend`.
- **cli:** `tynd plugin-add <pkg>` one-shot plugin installer — runs
  `bun add` and rewrites `tynd.config.ts::plugins` in a single pass.
  `--auto-runtime` flips `runtime:` to the matching `-custom` value
  inline.
- **cli:** `tynd plugin-check [dir]` validates a plugin package before
  publish — `package.json` shape, required files, native crate layout
  (`Cargo.toml [lib] = rlib`, `tynd_plugin_sdk` dep, `register_plugin!`,
  `TyndPlugin` impl). `--json` for CI.
- **cli:** `tynd plugin-list` inspects plugins declared in
  `tynd.config.ts`, showing kind (`js` / `native` / `bundled` /
  `missing`) plus declared capability allow-list.
- **cli:** runtime-plugin compatibility pre-flight — declaring a native
  plugin while the runtime is `lite` / `full` (instead of `*-custom`)
  fails `tynd dev` + `tynd build` immediately.
- **plugin-sdk:** `validate(schema, args)` helper — short-circuits bad
  plugin args before the round-trip to Rust. Accepts any Standard Schema
  v1 validator (valibot, zod, arktype); throws `PluginArgsError`.
- **cli:** `tynd doctor` prints a per-check status for Bun version,
  platform, project config, host binary, GitHub reach and the managed
  Rust toolchain. `--json` for automation.
- **cli:** `log.fail(message, { hint?, docs? })` helper consolidates
  the `log.error + log.hint` pair.
- **cli:** atomic binary writes — `tynd build` writes to a `.tmp` next
  to the target then renames, so a SIGTERM mid-build never leaves a
  half-written executable.
- **cli:** `ensureRustToolchain` installs a pinned Rust at
  `~/.tynd/toolchain/` for custom builds, even when the user already
  has a system `rustup`.
- **cli:** `exec.secretArgs` masks password argv (signtool `/p`,
  notarytool `--password`) in debug logs.
- **cli:** opt-in macOS code signing step in the release workflow,
  gated on `APPLE_CERT_P12_BASE64` / `APPLE_IDENTITY` secrets.

### Fixed

- **lite:** hot-reload path no longer panics on `Mutex` poison
  (`parking_lot::Mutex`, no poison state).
- **host:** `plugins.rs::register` releases the write lock before
  calling `init()` — no more self-deadlock when an `init` emits an
  event.
- **host:** `app/mod.rs "all"` branch uses `Vec::with_capacity` so the
  hot dispatch path skips a reallocation.
- **core:** `registerPlugin` throws on non-function method values and
  rejects `$` in plugin / method names (reserved for the wire format).
- **plugins/log:** `captureConsole` routes write failures to
  `console.error` so DevTools never goes silent when the file sink is
  unreachable.
- **plugins/theme:** `configure()` is HMR-safe — removes the prior
  `matchMedia` listener before registering a new one.
- **plugins/sql:** `Text` cells use `from_utf8_lossy` instead of
  silently collapsing to `""` on a single invalid byte.

### Documentation

- **GETTING_STARTED:** explicit Bun >= 1.3.0 prerequisite and a
  supported-platforms matrix (Win/macOS/Linux × x64/arm64).
- **docs site:** cut `v1.0` folder as the new latest; `v0.3`, `v0.2`,
  `v0.1` are archived and frozen.
- **NOTICE:** add section for the Tynd plugin native crates.

## [0.3.1] — 2026-04-21

### Fixed

- **ci:** add `libxss-dev` for `user-idle`'s XScreenSaver probe.
- **host:** macOS + Linux clippy nits (release profile).

## [0.3.0] — 2026-04-21

### Added

- **app:** `getFrameworkVersion` + `getBundleIdentifier`.
- **app:** `resourceDir` + `quitOnLastWindowClosed` flag.
- **backend:** `evalInFrontend(script, label?)`.
- **clipboard:** `onChange` polling monitor.
- **core:** log API with JSON-lines file + size rotation.
- **core:** frontend → backend events via `createFrontendEmitter`.
- **dialog:** `canCreateDirectories` option (macOS).
- **events:** `emitter.emitTo(label)` targets a single window.
- **fs:** file handle, symlink, hardlink, `copyDir`.
- **host:** `shell.revealInFolder` and `fs.trash`.
- **host:** native drag-drop events (`window:drop`, `window:drag-*`).
- **log:** `captureConsole` bridges `console.*` into `log.write`.
- **menu:** programmatic context menu (Windows).
- **monitor:** `fromPoint` + `cursorPosition`.
- **notification:** icon, sound, action buttons + `onAction`.
- **notification:** `checkPermission` + `schedule` + `cancel`.
- **power:** `power.getIdleTime` (cross-platform).
- **rpc:** `listenN` + `once` helpers for ad-hoc events.
- **rpc:** `withTimeout` + abortable helpers.
- **security:** opt-in capability ACL for `fs` + `http`.
- **tray:** `onEnter`/`onMove`/`onLeave` mouse events.
- **tray:** runtime `setIcon`/`setTooltip`/`setTitle`/`setMenu`/`setVisible`.
- **tray:** `setIconAsTemplate` (macOS).
- **updater:** periodic check + proxy + custom headers + `allowDowngrade`.
- **webview:** `userAgent` + `onNavigation` + `onPageLoad`.
- **window:** `findInPage` + `stopFindInPage`.
- **window:** `print`, `navigate`, `loadHtml`, `getUrl`.
- **window:** appearance polish (theme, bg, shadow, progress, badge,
  skip taskbar).
- **window:** `closeSelf` helper.
- **window:** cursor & mouse API (whole §6 category).
- **window:** `getTitle` getter.
- **window:** `getZoom` cached getter.
- **window:** `hideOnFocusLost` + `hideOnEscape` config flags.
- **window:** macOS titlebar transparent + fullsize content + traffic
  light inset.
- **window:** `modalTo` parent handle on create (Win + macOS).
- **window:** `positionAt(preset)` — 13-preset positioner.
- **window:** `setFocusable` + `setEnabled` (Windows).
- **window:** `setShadow` (macOS) + `setWindowIcon` runtime.
- **window:** `setVisibleOnAllWorkspaces` (macOS).
- **window:** Windows 11 Mica/Acrylic/Tabbed system backdrop.

### Fixed

- Audit-surfaced bugs + document RPC helpers.
- **menu:** cross-OS context menu (macOS + Linux).
- **notification:** action callbacks on all OS.

### Documentation

- **comparison:** mark window-scoped listeners supported via `emitTo`.
- **readme:** strip emoji from headings + tables.
- Scaffold v0.2, fix version picker dropdown.
- Sync README/RUNTIMES/API with log/power/security + all v3 window APIs.
- **v0.3:** cut new latest version with all v3 features.

## [0.2.0] — 2026-04-20

### Added

- **docs:** scaffold docs site on Next.js + Nextra with full SEO wiring.

### Fixed

- **cli:** auto-download `@tynd/host` binary when postinstall skipped.

### Documentation

- Rework site — custom Cards, theme sync, nav parity, LLMs sidebar.

## [0.1.0] — 2026-04-19

### Added

- 2nd-launch focus + argv forwarding.
- App APIs, OS info, dialogs, clipboard, menu accelerators.
- **cli:** `--cwd`/`--json` on info, `--dry-run` on clean, global
  `--verbose`/`--quiet`, typo suggestions.
- **cli:** scaffolding, dev mode, build pipeline.
- **cli:** `tynd start` to run app from prebuilt artifacts.
- **cli:** bundle sidecar binaries into the built `.exe`.
- **cli:** code signing + macOS notarization.
- **cli:** verbose mode propagates to host, smarter binary hint.
- **cli:** installer bundles via `tynd build --bundle`
  (app/dmg/deb/rpm/appimage/nsis/msi).
- **cli:** invalidate cache on CLI version bump + watch `package.json`.
- **cli:** `keygen` + `sign` for the auto-updater.
- **cli:** multi-size icon generation per OS.
- **cli:** prefix child process logs with `[host]` / `[vite]` for clarity.
- **cli:** prompt before installing deps + pin runtime versions.
- **cli:** rebuild in `tynd start`; square SVG icons; NSIS per-user by
  default.
- **cli:** reorder runtime prompts, update outdated hints.
- **cli:** runtime-validate `tynd.config` with valibot.
- **cli:** watch `tynd.config.ts` + log reload duration.
- **cli:** wire up `log.debug`, TTY/`NO_COLOR` detection, spinners +
  unified error hints.
- **core:** typed RPC client and app lifecycle.
- **core:** re-export Web-platform globals from `@tynd/core/client`.
- **core:** route binary client APIs through `tynd-bin://`.
- **core:** streaming IPC via `AsyncGenerator`.
- Deep-link URL schemes (`myapp://`).
- **dev:** hot-reload backend without tearing down the WebView.
- `fs.watch` + global keyboard shortcuts.
- **full:** `tynd-full` runtime launcher — Bun subprocess backend.
- **host-rs:** native WebView host library.
- **host:** `@tynd/host` — npm wrapper for native binaries.
- **host:** `compute.randomBytes` — CSPRNG for lite + full parity.
- **host:** `singleInstance` API to prevent dual-launch.
- **host:** process / fs / store / os OS APIs (lite + full parity).
- **host:** embedded SQL via `rusqlite` (lite parity with `bun:sqlite`).
- **host:** HTTP client, sidecar, PTY terminal, binary fs + shared
  event bus.
- **host:** native compute + workers for lite/full perf parity.
- **host:** opt-in crash reporter (panic → file).
- **host:** `TYND_LOG` env-gated logger (info/warn/error/debug).
- **host:** WebSocket client via `tungstenite` (lite/full parity).
- **host:** zero-copy binary IPC via `tynd-bin://` custom scheme.
- Keyring, CSP, webview reload / zoom / devtools.
- **lite:** hot-reload backend in QuickJS mode without tearing down the
  WebView.
- **lite:** `tynd-lite` runtime launcher — embedded QuickJS backend.
- **lite:** Web-standards polyfill layer.
- Multi-window support via `tyndWindow.create`.
- **playground:** example demo + generic LLM chatbot.
- **updater:** install + relaunch on Windows and Linux.
- **updater:** manifest check + signed download.
- Validate every external input with valibot + trace hot paths with
  `log.debug`.
- Window events via `tyndWindow`.
- Window geometry, monitors, autolaunch.

### Changed

- **cli:** Bun-first file APIs in config + pkg loaders.
- **cli:** use `Bun.hash` / `Bun.write` / `Bun.CryptoHasher` on hot paths.
- **core:** inline runtime check so Bun DCE drops the dead branch.
- **host:** bounded worker pool for OS dispatch (no more spawn-per-call).
- **host:** enable WAL + `synchronous=NORMAL` on on-disk SQL opens.
- **host:** pre-serialize OS events to skip a `Value` clone.
- **host:** replace websocket busy-poll with blocking `read_timeout`.
- **host:** swap `Mutex<HashMap>` registries to `DashMap`.
- **host:** swap `std::sync` `Mutex`/`Condvar` to `parking_lot`.
- Migrate compression from gzip to zstd + bump deps.
- **cli:** swap all remaining file IO to `Bun.file` / `Bun.write`.
- **cli:** use `Bun.semver.order` instead of hand-rolled comparator.
- **core,cli:** split `client.ts` + extract `pack.ts` and spawn-helpers.
- **core:** trim non-essential public surface.
- **host:** drop dead JSON binary paths in `fs` + `compute`.
- **host:** rename custom scheme from `bv://` to `tynd://`.
- Share TYNDPKG reader + isolate dev reload machinery.
- Strip decorative separators and extract menu/tray modules.
- Trim narrative comments, keep only the WHY.

### Fixed

- 17 bugs across host / core / cli.
- Bulletproof streaming RPC at any yield rate.
- **ci:** restore Rust builds on all runners.
- **cli:** `create` rolls back on scaffold failure + `init` bootstraps
  `package.json`.
- **cli:** default Angular `outputPath` to `dist/<project>[/browser]`.
- **host:** add typecheck — previously skipped due to missing tsconfig.
- **host:** register `tynd-bin` in dev mode, offload IO to `call_pool`.

### Documentation

- Add FRAMEWORKS with per-framework support matrix.
- Add SIGNING covering Windows, macOS and Linux workflows.
- Clarify Bun prerequisite across READMEs.
- **comparison:** add 7 missing categories and refresh feature tables.
- Document `compute` + `workers`; add Concurrency comparison section.
- Document `randomBytes`, websocket, sql APIs across docs.
- Document `tynd://` rename and `tynd-bin://` binary scheme.
- Drop stack jargon from user-facing docs, surface phase 5-10 features.
- Emoji section anchors + banned unicode arrows purged.
- Overhaul around lite/full philosophy + comprehensive catalog.
- README, runtime guide, framework comparison.
- Refresh binary sizes, highlight Bun JIT, hide engine name.
- **runtimes:** drop forward-looking mentions of a `tynd bench` command.
- **runtimes:** mark `tynd-bin` routes in the OS API table.
- Split API reference into API; document new OS APIs.
- Sync COMPARISON + GETTING_STARTED with `singleInstance`,
  `crashReporter`, tests.
- Sync NOTICE, getting-started, contributing with current state.
