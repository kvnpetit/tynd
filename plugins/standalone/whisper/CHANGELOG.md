# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [1.0.0] — Unreleased

First stable release.

### Added

- Local speech-to-text via `whisper-rs` (whisper.cpp), behind the `inference`
  feature. Accelerator features: `cuda`, `metal`, `vulkan`, `hipblas`,
  `openblas`.
- Language auto-detection, `detectLanguage` with its probability, `cancel`
  that aborts the native decode, and `transcribeStream` that honours its
  `signal`. Segments are emitted once the buffer is decoded.
- A 15 s "busy" timeout, `threads` clamped to the available parallelism, and
  stream cancellation and model release at app exit.
- Native crate `tynd-plugin-whisper`. Marked experimental and published off
  npm `latest`.
