# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [1.0.0] — Unreleased

First stable release.

### Added

- OS credential storage through the host `keyring` API: Keychain Services
  (macOS), Credential Manager (Windows), Secret Service (Linux).
- `set`, `get` and `delete` keyed by `{ service, account }`. `get` returns
  `null` for an absent entry.
- Requires `@tynd/core` as a peer dependency.
- Imported from `@tynd/plugin-keyring/client`.
