# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [1.0.0] — Unreleased

First stable release.

### Added

- Native `tynd` and `tynd-mcp` executables (Rust, `crates/cli`), shipped
  with `tynd-bundler` through six `@tynd/tooling-<os>-<arch>` platform
  packages. `@tynd/cli` and `@tynd/mcp` are thin npm launchers; Bun runs only
  the application backend.
- `@tynd/mcp`: a Model Context Protocol server that discovers running
  `tynd dev` apps and exposes tools, resources and prompts to AI agents.
- A dependency-free `defineConfig` facade with literal inference, editor
  completion and environment-aware async config factories. Rust owns config
  loading, validation, project discovery, process supervision, builds,
  publishing and MCP.
- Layered config: base, environment, local and environment-local files,
  project dotenv files, and recursive local/npm `extends` with cycle detection
  (remote config URLs are rejected). `tynd.config.json` accepts `$schema`.
- Plugin system: `@tynd/plugin-sdk` and the `tynd-plugin-sdk` Rust crate with
  proc-macros, for JS-only or native plugins. Native plugins and selected host
  APIs produce an app-specific Rust host with deterministic generated source;
  JavaScript plugins are injected into the backend bootstrap.
- Official plugins, in two groups: host-backed (`@tynd/plugin-fs`, `-http`,
  `-process`, `-keyring`, `-notification`, `-shortcuts`, `-vault`,
  `-websocket`, ...) and standalone (`audio`, `oauth`, `sql`, `terminal`, `tts`).
- CLI commands: `create`, `init`, `add`, `remove`, `dev`, `start`, `test`,
  `build`, `clean`, `upgrade`, `validate`, `doctor` (with `--fix`), `icon`,
  `info`, `size`, `explain`, `config show`, `plugin new|check|dev|list|types`,
  `updater keygen|sign|publish|release|server`, `report`, `profile` and
  `completions`. Output paths use `--out`/`-o`.
- `tynd dev` resolves the pinned Bun executable, waits for the frontend
  server, supervises the host and reloads backend, config, dotenv and native
  plugin sources, compiling a new host before replacing the running one.
- Host capabilities: persisted window state, shortcuts, deep links (any
  `scheme://` URL), biometric prompts, screenshots, system stats, networking,
  vault storage, GPU controls and child WebViews.
- Windows: `tyndWindow` state, geometry, kiosk, simple fullscreen, opacity,
  always-on-top levels, badges, taskbar progress, content protection, cursor
  control and `positionAt` presets (tray-aware). `setEffects` applies Mica,
  Tabbed, Acrylic and Blur on Windows and the macOS materials, and rejects on
  Linux. Frameless windows use `data-tynd-drag`, `data-tynd-no-drag` and
  `data-tynd-resize`; `titleBarStyle: "overlay"` keeps the macOS traffic
  lights. Native file drop gives real paths; `startDrag` and
  `data-tynd-drag-file` drag files out to other apps.
- `@tynd/plugin-multiwindow`: secondary windows with handles (from the
  frontend, or the backend through `getWindow`), typed messages between
  windows, a per-window menu bar (Windows and Linux; macOS has one app-wide
  bar), `modalTo`, popover options, and child webviews with their own label,
  bounds, data partition and capability identity.
- Backend window events: `getWindow(label)` handles take the `on*` window
  events, and `onCloseRequested` can prevent a user close of any window.
- Menus (`@tynd/plugin-menu`): menu bar with icons, native roles,
  `defaultAppMenu` and runtime mutation by id; programmatic and declarative
  (`data-tynd-contextmenu`) context menus. `app.dock.*` (macOS: menu, bounce,
  icon, hide), `app.setJumpList` (Windows) and recent documents (Windows,
  macOS).
- Tray (`@tynd/plugin-tray`): up to 16 icons with handles, runtime menu
  mutation, pointer events with icon bounds, and `attachWindow` popover
  windows on Windows and macOS (rejects on Linux).
- Dialogs: file pickers with `grantAccess`, and `message` with custom buttons,
  `defaultId` / `cancelId` and a checkbox.
- Notifications: images, sounds, urgency, timeout, action buttons and
  categories, inline reply (Windows, macOS), click / action / reply / close
  events, scheduling, and delivered-notification management (Windows,
  Linux).
- `tyndWebview`: static per-webview web `permissions`, cookies, `evaluate`,
  `clearData`, `printToPdf`, `capturePage`, dialog and silent `print`
  (silent print is not on macOS; `getPrinters` is Windows-only), audio mute
  (not on macOS), proxy, and `newWindow` / `downloads` policies with events.
- Custom protocols: the asset protocol (`convertFileSrc`, Range seeking,
  `assets.scope` globs) and backend-served schemes with `app.protocol`.
- IPC: every call handle has `cancel()`, `api.withOptions({ signal, timeout })`
  cancels or bounds a call, and the handler's `getCallContext().signal`
  aborts; `Channel<T>` pushes values to the caller; `app.use` middleware.
  Binary RPC values travel as base64 in frames of up to 8 MiB; `fs` binary
  I/O uses `tynd-bin://`.
- `app.crashReporter`: local JSON reports for host panics, backend exits and
  renderer losses (nothing is uploaded), and `app.on("render-process-gone")`
  on Windows and macOS.
- Security: native renderer commands are denied unless a capability grants
  them (per window, webview, origin and platform); renderer file access is
  limited to the app's own folders by default, with protected locations
  (SSH, cloud credentials, browser profiles, keychains) no rule can reach;
  backend runtime grants (`capabilities.allowPath`, `allowUrl`), optionally
  persisted; per-webview IPC token checked on every message; auto-injected
  CSP on bundled pages.
- Packaging targets: NSIS, MSI and MSIX on Windows; `.app` and DMG on macOS;
  AppImage, deb, rpm, Flatpak, Snap and PKGBUILD on Linux. Builds target the
  machine they run on (no cross-compilation or universal macOS binary).
- Every packed executable carries a BLAKE3 integrity digest checked at start.
- `tynd icon` renders `.png`, `.ico`, `.icns`, Linux sizes and MSIX tile
  assets from one SVG or square PNG.
- Updater: Ed25519-signed manifests and artifacts, bsdiff patch updates
  verified after reconstruction, release channels, staged rollout
  percentages and key rotation.
- Not included in 1.0.0: mobile, headless/server mode, HTTP/2 in the `http`
  plugin (use `fetch`), request interception, basic-auth and certificate
  handlers, a runtime permission-request handler, download progress or
  cancellation, Windows thumbnail toolbar and tray balloons.
- macOS 14 (Sonoma) is the minimum supported macOS: `.app` bundles declare
  `LSMinimumSystemVersion` 14.0 and webview `proxy` works on every macOS build.
- Windows installers support Evergreen, offline, skipped and fixed WebView2
  distribution modes.
- Updater: backend-only `updater.proxy` and `updater.headers` config for
  private update servers (`${env:NAME}` expanded at bundle time; headers go
  only to the configured endpoint origins). `tynd updater publish` always writes a
  `channel` into the manifest (default `stable`).
- Window `downloads` option (`"allow"` | `"deny"`); by default only app-local
  content can download app-local or `blob:` URLs.
- `tynd build --debug-protocol` to drive a packaged binary with `@tynd/test`.
- `store`: `has`, `values`, `entries`, `length` and `reload`.
- Every backend runs on Bun. A frontend build tool is required:
  `tynd create` starters are `react` (default), `vue`, `svelte`, `solid`,
  `preact`, `lit` and `angular`; `tynd init` needs a detected frontend tool
  or an existing Tynd config.
- Each app compiles its own host with cargo from `@tynd/host-src`.
- Plugins are installed per API: plugin-first APIs (`fs`, `http`, `process`,
  `keyring`, `notification`, `shortcuts`, ...) are imported from
  `@tynd/plugin-<api>/client` and declared in `tynd.config` `plugins`.
  `tynd add <api>` installs the package; `tynd validate` reports an import
  that is not available.
- Security model: calls from a remote origin need an explicit capability
  grant; OS and plugin events reach app-local webviews only; resources a
  renderer creates (watches, sockets, processes, shortcuts, ...) belong to the
  webview and frame origin that created them; the updater trusts only
  backend-supplied configuration and verification keys.
- With no `security` policy, the HTTP URL scope is open, filesystem access
  remains app-scoped, and process execution requires `processAllowedBinaries`.
  `tynd validate` and `tynd build` warn when no policy is configured. HTTP
  allow/deny rules match the normalized URL.
- Spawned processes refuse loader, shell, interpreter and Git environment
  variables; `process.execShell` refuses a caller-supplied `env`.
- `shell.openPath` refuses launcher, script, installer, disk-image and
  shortcut files.
- The debug protocol is off in packaged apps unless built with
  `--debug-protocol`.
- `tynd doctor` and `tynd info` are project-scoped: they query no registry,
  system WebView or network service.
- `store` files live at `<config_dir>/<app-id>/<namespace>/store.json`.
- `power.on*` return a promise; `setBadge` and `setEnabled` reject where
  unsupported; `http.download` fails on a non-2xx status.
- Apps that set their own CSP need `worker-src blob:` for `workers.spawn`.
- `binaryArgs` and a packaged app's own arguments reach the backend after
  `--` in `process.argv`.
- The documentation site is https://kvnpetit.github.io/tynd/, and the config
  schema is served at
  `https://kvnpetit.github.io/tynd/schemas/tynd.config.json`.
- MSRV is Rust 1.95.
