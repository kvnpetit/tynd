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
  `-websocket`, ...) and standalone (`audio`, `command-palette`, `gpu`,
  `http-server`, `llm`, `oauth`, `screen-rec`, `sql`, `terminal`, `tts`,
  `wgpu`, `whisper`). `llm`, `whisper`, `screen-rec` and `wgpu` are
  experimental and published under the npm `experimental` dist-tag.
- CLI commands: `create`, `init`, `add`, `remove`, `dev`, `start`, `test`,
  `build`, `clean`, `upgrade`, `validate`, `doctor` (with `--fix`), `info`,
  `size`, `explain`, `config show`, `plugin new|check|dev|list|types`,
  `updater keygen|sign|publish|release|server`, `report`, `profile` and
  `completions`. Output paths use `--out`/`-o`.
- `tynd dev` resolves the pinned Bun executable, waits for the frontend
  server, supervises the host and reloads backend, config, dotenv and native
  plugin sources, compiling a new host before replacing the running one.
- Host capabilities: persisted window state, shortcuts, deep links (any
  `scheme://` URL), biometric prompts, screenshots, system stats, networking,
  vault storage, GPU controls and child WebViews.
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
- With no `security` policy, `fs`, `http` and `process` stay open, but
  `tynd validate` and `tynd build` warn. HTTP allow/deny rules match the
  normalized URL.
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
- The documentation site is https://tynd-apps.github.io/tynd/, and the config
  schema is served at
  `https://tynd-apps.github.io/tynd/schemas/tynd.config.json`.
- MSRV is Rust 1.95.
