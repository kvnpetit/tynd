# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [1.0.0] — Unreleased

First stable release.

### Added

- Native crate `tynd-plugin-llm` with an `inference` feature gating
  `llama-cpp-2`; the default build is a stub that returns a "rebuild with
  --features inference" hint. Accelerator features: `cuda`, `metal`, `vulkan`.
- Methods: `load`, `generate`, `complete`, `unload`, `list`. `generate`
  streams `llm:token` events and ends with one `llm:end` event; the client
  exposes it as an `AsyncIterable<string>` with an optional `AbortSignal`.
- UTF-8-safe token streaming, chunked prompt decoding, `stop` sequences that
  are never streamed, a 15 s "busy" timeout, and stream cancellation and model
  release at app exit.
- Marked experimental and published off npm `latest`.
