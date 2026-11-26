# Changelog

This file tracks workspace-wide changes (tooling, shared configs, monorepo layout). Per-plugin
release notes live in each plugin's own `CHANGELOG.md` (e.g., `audio/CHANGELOG.md`,
`sql/CHANGELOG.md`) since plugins version and publish independently.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [1.0.0] — Unreleased

First stable release.

### Added

- Standalone official plugins workspace: `audio`, `oauth`, `sql`,
  `terminal` and `tts`.

### Removed from the pre-release catalog

- Build command palettes in the app's frontend to replace `command-palette`.
  Use WebGPU and WebGL capability checks to replace `gpu`.
- Replace `http-server` with `Bun.serve` in the backend. Use `createBackend`
  for calls from a Tynd WebView to its own backend.

[Unreleased]: https://github.com/kvnpetit/tynd/compare/v1.0.0...HEAD
[1.0.0]: https://github.com/kvnpetit/tynd/releases/tag/v1.0.0
