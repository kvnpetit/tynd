# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [1.0.0] — Unreleased

First stable release.

### Added

- Audio capture and playback via `cpal` (CoreAudio / WASAPI / ALSA), behind
  the `capture` feature; both share device enumeration.
- WAV (RIFF) read/write via `hound`, behind the `wav` feature.
- Native crate `tynd-plugin-audio`. Host shutdown drops every live stream on
  its audio thread and joins that thread.
