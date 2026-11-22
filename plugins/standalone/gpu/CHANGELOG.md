# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [1.0.0] — Unreleased

First stable release.

### Added

- `gpu.info()`: cached capability snapshot (vendor, renderer,
  dedicated/integrated heuristic, WebGL1/WebGL2/WebGPU support, adapter
  features and limits). Probe WebGL contexts are released after use.
- `gpu.requireWebGPU()` / `gpu.requireWebGL2()`: readiness checks that throw
  with a printable diagnostic.
- `gpu.benchmark()`: WebGPU 256x256 matmul with a WebGL fallback, returning
  ops/sec, a 0-100 score and a `low | medium | high | extreme` category in
  about 500 ms.
- `gpu.supportsFeature(name)` and `gpu.memoryStatus()` (best effort, via
  `navigator.deviceMemory` and `performance.memory`).
- Pure JS, zero native deps.
