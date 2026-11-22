# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [1.0.0] — Unreleased

First stable release.

### Added

- Native crate `tynd-plugin-screen-rec` with a `capture` feature gating
  `scap`; the default build is a stub that returns "rebuild with
  `--features capture`" errors.
- Methods: `listSources`, `hasPermission`, `requestPermission`, `start`,
  `stop`, `pause`, `resume`, `list`. `listSources` enumerates displays and
  windows, and a display ID can be passed as `RecordingOpts.monitor`.
- Per-platform permission notes (macOS prompt, Windows, Linux PipeWire
  portal vs X11).
- Marked experimental and published off npm `latest`: `start()` does not yet
  write frames to disk, and `pause` / `resume` only adjust the duration.
