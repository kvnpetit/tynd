# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [1.0.0] — Unreleased

First stable release.

### Added

- `@tynd/core/client/bridge`: `osCall`, `osOn`, `osDeadline`, the `bin*`
  helpers, base64 and window-label helpers and `decodeIpcError`, the public
  contract plugin clients build on.
- `@tynd/core/plugin` is isomorphic (no Bun or Node imports): `definePlugin`,
  `createPluginClient` and the plugin types. `@tynd/core/testing` holds
  `installTyndMock`.
- `definePlugin`, with the same canonical name and method validation as the
  native host.
- `onFrontendEmit` and `clearFrontendListeners()`; the backend clears the
  listeners on close.
- Backend: an `app:beforeStart` hook that does not finish within 3 s is
  abandoned with a warning. These hooks run before the host-call transport
  exists, so they must not call host APIs.
- Inline capability permissions accept the versioned Rust wire shape.
- Plugin-first APIs (`fs`, `http`, `process`, `keyring`, `notification`,
  `shortcuts`, `vault`, `websocket`, ...) are imported from
  `@tynd/plugin-<api>/client`, not from `@tynd/core/client`.
- Client `AppInfo` has no `version`: the host accepts it only from the
  backend.
- Updater `autoCheck`, `checkOnStartup` and `autoDownload` require the main
  window to import `@tynd/plugin-updater/client`.
