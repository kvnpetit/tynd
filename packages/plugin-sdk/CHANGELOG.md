# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [1.0.0] — Unreleased

First stable release.

### Added

- **JS SDK:**
  - One model per plugin shape: native plugins export
    `defineTypedPlugin<C>(name)` (typed RPC proxy over the generated
    `*.gen.ts`) from `client.ts`; pure JS plugins pair a `backend.ts`
    (`definePlugin`) with a `client.ts` (`createPluginClient`).
  - `defineTyndPlugin({ name, methods?, hooks? })` from
    `@tynd/plugin-sdk/hooks`: awaitable lifecycle hooks (`app:beforeStart`,
    `app:ready`, `app:beforeQuit`, `window:created`, `window:closed`,
    `ipc:before`, `ipc:after`).
  - `createPluginClient`, `onHostEvent` and `createHandleEvents` (native
    plugin events, received on the OS event bus through `osOn`), and `validate(schema, args)` with
    `PluginArgsError` (Standard Schema v1).
- **Rust SDK** (`tynd-plugin-sdk`):
  - `TyndPlugin` trait (`name`, `init`, `dispatch`, `dispatch_rkyv`,
    `shutdown`, `command_names`) and `register_plugin!`.
  - `PluginCx` with `emit`, `os_call`, `spawn_interval` and capability gating
    (`with_allowed_apis` denies all by default;
    `with_allowed_apis_unrestricted()` opts out), plus test builders
    (`PluginCx::test()`, `test_cx()`, `EventCapture`).
  - `BackgroundHandle`, `Registry<T>`, `ArgsExt` (`require` / `opt`),
    `PluginError` / `PluginResult`, `unknown_method`, and the
    `rkyv_decode_args` / `rkyv_encode_result` helpers.
  - Re-exports of `tynd_plugin`, `rkyv`, `specta`, `specta_typescript` and
    `specta_serde`, so plugins need no direct dependency on them.
- **Proc-macros** (`tynd-plugin-macros`):
  - `#[tynd_plugin::commands]`, `#[tynd_plugin::skip]`,
    `#[tynd_plugin::plugin]` + `#[plugin_impl]` + `#[command]`,
    `#[plugin_command(zero_copy)]` and `gen_types_bin!`.
  - Generated TypeScript types follow serde renames (camelCase fields).
  - Untagged `plugin_init` / `plugin_shutdown` methods implement the
    `TyndPlugin` lifecycle hooks without exposing renderer commands.
  - The generated command list backs fail-closed renderer permissions
    (`allow-*`, `deny-*`, `allow-all`, `deny-all`, `default`).
  - Plugin names must match `^[a-z][a-z0-9-]{0,30}$`; unknown
    `#[plugin_command]` flags fail compilation.
