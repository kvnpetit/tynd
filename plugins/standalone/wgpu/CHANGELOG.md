# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [1.0.0] — Unreleased

First stable release.

### Added

- Native crate `tynd-plugin-wgpu` with a `native` feature gating `wgpu`,
  `tao`, `raw-window-handle` and `pollster`; the default build is a stub that
  returns a "rebuild with --features native" hint.
- Methods: `available`, `createSurface`, `resize`, `close`, `list`, `submit`
  (stub). Each surface is a `tao::Window` with a `wgpu::Surface` whose event
  loop emits `wgpu:frame`, `wgpu:resize`, `wgpu:close` and `wgpu:error`.
  Sizes are validated; device errors become `wgpu:error` events.
- `available()` reports `false` with a reason on macOS and Linux.
- TypeScript `WgpuSurface` client with `onFrame` / `onResize` / `onClose`
  and a Three.js adapter scaffold (`ThreeNativeRenderer`) whose `render()` is
  a stub.
- Marked experimental and published off npm `latest`.
