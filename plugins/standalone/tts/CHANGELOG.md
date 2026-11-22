# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [1.0.0] — Unreleased

First stable release.

### Added

- Text-to-speech via the `tts` crate (SAPI 5 on Windows,
  AVSpeechSynthesizer on macOS, speech-dispatcher / espeak on Linux), behind
  the `synthesis` feature.
- Voice enumeration; `rate` and `pitch` (0..2, 1 = normal) and `volume`
  (0..1) mapped onto each backend's range.
- `speak()` queues behind the current utterance; `stop()` interrupts.
- Native crate `tynd-plugin-tts`.
