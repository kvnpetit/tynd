# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [1.0.0] — Unreleased

First stable release.

### Added

- Embedded HTTP + WebSocket server on a Bun backend built on `Bun.serve`.
- `httpServer.serve(opts, handlers)` with a `fetch` handler, request
  streaming and `websocket` lifecycle hooks.
- `setCookieHeader()` carries `Set-Cookie` values from webview handlers,
  where the browser drops that header name; a value with CR, LF or NUL is
  rejected with a 500.
- Bind validation in the core host: loopback hosts need `network-loopback`,
  any other host (including look-alike DNS names) needs `network-public`.
