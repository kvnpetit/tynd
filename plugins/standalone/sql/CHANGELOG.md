# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [1.0.0] — Unreleased

First stable release.

### Added

- SQLite through Bun's embedded `bun:sqlite`, with typed query and execute
  helpers in the plugin client.
- Filesystem-policy checks through the Bun-to-host bridge. `ATTACH`,
  `VACUUM INTO`, `load_extension()` and the `temp_store_directory`,
  `data_store_directory` and `lock_proxy_file` pragmas are blocked.
