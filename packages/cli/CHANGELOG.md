# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [1.0.0] — Unreleased

First stable release.

### Added

- Native Rust `tynd` (`crates/cli`) covering project, plugin, updater,
  release, profile, completion and diagnostic commands: `create`, `add`,
  `dev`, `start`, `test`, `build`, `init`, `clean`, `validate`, `upgrade`,
  `config show`, `info`, `explain`, `size`, `doctor`, `remove`,
  `updater keygen`, `updater sign`, `updater publish`, `updater release`,
  `updater server`, `report`, `plugin new`, `plugin check`, `plugin dev`,
  `plugin list`, `plugin types`, `profile` and `completions`.
- `tynd create` starters: `react` (default), `vue`, `svelte`, `solid`,
  `preact`, `lit` and `angular`.
- Typed `defineConfig` facade with no runtime dependency, including
  environment-aware async config factories and capability helpers.
- Layered base/env/local config loading, dotenv support, safe local/npm
  `extends`, pinned Bun resolution, frontend readiness, backend reload, native
  plugin host generation, fixed WebView staging and distribution signing.
- `add`/`remove` manage plugins, `doctor --fix` repairs, and output paths use
  `--out`/`-o`.
- The npm package launches the matching native binary from `@tynd/tooling`,
  whose optional platform packages cover macOS, Linux and Windows on x64 and
  ARM64; it keeps TypeScript only for config authoring.
- Every backend runs on Bun. A frontend build tool is required; `tynd init`
  needs a detected frontend tool or an existing Tynd config.
- Plugins are installed per API: `tynd add <api>` installs the matching
  `@tynd/plugin-<api>` package.
- `doctor` and `info` are project-scoped and run no WebView, registry, network
  or global toolchain checks. Missing optional tools are reported only by the
  command that needs them.
- Plugin, build, publish, updater server and cleanup paths enforce bounded
  input, path containment, symlink/reparse safety and secret redaction.
