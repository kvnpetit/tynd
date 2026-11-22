# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [1.0.0] — Unreleased

First stable release.

### Added

- Native Rust MCP server (`tynd-mcp`, `crates/cli`) with 34 tools, static
  and dynamic resources, six prompts, app discovery, debug transport and
  bounded payloads. It detects running `tynd dev` apps and serves AI agents
  over stdio.
- Lazy runtime attachment, child-process reaping, protocol-safe shutdown and
  packaged framework/plugin catalog discovery.
- `@tynd/mcp` is a small launcher for the platform `tynd-mcp` executable.
- `tynd-mcp call webview.evaluate` requires `--allow-eval`, like `serve`.
