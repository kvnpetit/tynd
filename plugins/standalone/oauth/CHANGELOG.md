# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [1.0.0] — Unreleased

First stable release.

### Added

- One-shot localhost OAuth callback receiver on an optional Bun backend; the
  core host validates the loopback bind.
- `oauth.start({ port?, path?, successHtml? })` returns a flow handle with
  `wait()` / `cancel()` and state validation.
- A default success page that closes the browser tab after the redirect.
- `oauth:callback` and `oauth:error` events; the listener stops after one
  request.
