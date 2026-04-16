# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository. It is checked in so every contributor gets the same context.

## Workflow — every task

Follow this loop for every non-trivial change. Skip steps only when truly N/A (e.g. doc-only edit → no build).

1. **Design** — restate the goal, identify affected packages (TS/Rust/both), note IPC/format/cache impacts. For larger work, write a short plan.
2. **Develop** — make the edits. Touch the smallest surface that solves the problem. Don't add abstractions for hypothetical future needs.
3. **Verify** — run the checks below until clean:
   - **Typecheck (TS)**: `bun run typecheck` from repo root (runs `tsc --noEmit` across all `@vorn/*` workspaces via filter). For a single package: `bunx tsc --noEmit -p packages/<pkg>`.
   - **Biome (TS lint + format)**: `bun run check` (lint + format check). To auto-fix: `bun run format` + `bun run lint --write`. CI runs `bun run ci` (= `biome ci`, no writes).
   - **Rust check + lint**: `cargo check --workspace --all-targets` then `cargo clippy --workspace --all-targets -- -D warnings`.
   - **Rust format**: `cargo fmt --all -- --check`.
   - There is no test runner; if behavior matters, exercise it via a playground app under `playground/`.
4. **Review modifications** — `rtk git status` then `rtk git diff` (per RTK rule). Read every hunk; delete dead code, stray comments, debug prints. Confirm no unrelated files changed.
5. **Build** — produce the artifacts the change actually affects:
   - TS-only change: `bun run --cwd packages/cli build` (rebuilds the CLI bundle).
   - Rust change: `cargo build --release -p vorn-full` and/or `cargo build --release -p vorn-lite` (and `-p vorn-host` only matters transitively).
   - End-to-end packaging change: run `vorn build` against a `playground/` app and inspect the produced `release/<name>[.exe]`.

If a step fails, fix the root cause — do not skip checks or `--no-verify`.

## Code style

- **Keep it simple.** Prefer straightforward code over clever abstractions. Don't add layers, helpers, or generics that the current task doesn't require.
- **Comments and docs must earn their place.** Default to none. Only write a comment when the *why* is non-obvious — a hidden constraint, a subtle invariant, a workaround for a specific bug, or behavior that would surprise a reader. Don't narrate *what* the code does; well-named identifiers already do that.
- **No rot-prone comments.** Don't reference the current task, callers, or history ("used by X", "added for Y", "see issue #123"). Those belong in commit messages / PRs.
- **No speculative error handling.** Only validate at system boundaries (user input, external APIs, IPC payloads). Trust internal code.
- **Don't reimplement what already exists.** Before writing a new helper/util/function, grep the touched package (and `packages/core`, `packages/cli/src/lib`, `packages/host-rs/src`) for existing equivalents. Reuse or extend the existing one instead of creating a parallel version.
- **Check if a refactor is warranted first.** If the existing code is close but not quite right, prefer refactoring/extending it over adding a sibling. If you spot duplicated logic (same code in 2+ places, or near-duplicates that drifted), flag it and consolidate as part of the change — don't add a third copy.

## Project

**Vorn** — desktop app framework like Tauri/Wails but with a **TypeScript backend** (no Rust/Go required from app authors). Native window via `wry` + `tao`. Two backend runtimes share the same TS API:

- **`full`** — spawns a Bun subprocess; backend talks to the Rust host over stdin/stdout JSON.
- **`lite`** — embeds QuickJS (`rquickjs`) inside the Rust binary; backend runs in-process, no Bun.

## Repository layout

Hybrid Bun + Cargo monorepo. The same `packages/` directory holds both worlds:

| Package | Kind | Crate / npm name | Role |
|---|---|---|---|
| `packages/host-rs` | Rust lib | `vorn-host` | wry/tao event loop, IPC bridge, OS APIs (dialog, clipboard, notification, tray, menu, window) |
| `packages/full` | Rust bin | `vorn-full` | Spawns Bun subprocess; links `vorn-host` |
| `packages/lite` | Rust bin | `vorn-lite` | Embeds QuickJS; links `vorn-host` |
| `packages/host` | npm | `@vorn/host` | Postinstall (`install.ts`) downloads pre-built `vorn-full`/`vorn-lite` binaries from GitHub Releases |
| `packages/core` | npm | `@vorn/core` | Backend (`./`) and frontend (`./client`) APIs — `app.start`, `createEmitter`, `createBackend`, OS proxies |
| `packages/cli` | npm | `@vorn/cli` | `vorn` CLI: create/dev/build/init/clean/validate/upgrade/info |

Workspaces: `package.json` declares `packages/*`, `playground/*`. `Cargo.toml` declares only the three Rust crates.

`packages/cli` has a quirk: its `exports` field exposes `src/lib/config.ts` (so apps can `import type { VornConfig } from "@vorn/cli"`), but the runtime entry is `bin/index.js` (built from `src/index.ts`).

## Common commands

```bash
# JS side (workspaces — run from repo root unless noted)
bun install
bun run --cwd packages/cli dev            # run CLI from source
bun run --cwd packages/cli build          # bundle CLI → packages/cli/bin/index.js

# Rust side (cargo workspace)
cargo build -p vorn-full --release
cargo build -p vorn-lite --release
cargo build -p vorn-host                  # library only — no binary

# CLI in dev (point at a playground app)
bun run --cwd packages/cli dev -- dev --cwd ../../playground/react-full
```

The CLI auto-discovers binaries in this order: workspace `target/release` → `target/debug` → `node_modules/@vorn/host/bin/<plat>-<arch>` → PATH (`packages/cli/src/lib/detect.ts:findBinary`). For local dev you usually just `cargo build -p vorn-full --release` and the CLI finds it.

There is **no test suite** in this repo — don't invent test commands. Exercise changes via a `playground/` app. Lint + format is handled by **Biome** (`biome.json` at root) for TS/JSON, and `rustfmt` + `clippy` for Rust.

Linux build dependencies (match `.github/workflows/ci.yml` and `build-host.yml`):

```bash
sudo apt-get install -y \
  libgtk-3-dev \
  libwebkit2gtk-4.1-dev \
  libjavascriptcoregtk-4.1-dev \
  libsoup-3.0-dev
```

## Architecture — things that aren't obvious from one file

### IPC topology (full mode)

```
Frontend (WebView) ──postMessage──▶ Rust (wry ipc_handler) ──stdin JSON──▶ Bun backend
              ▲                                  ▲                                │
              │ evaluate_script                  └────── stdout JSON ─────────────┘
```

- `postMessage` body starting with `{"type":"call"` is forwarded to Bun **without re-parsing** (fast path in `host-rs/src/app.rs`). Other message types go through a single `serde_json` parse.
- Backend writes its window/menu/tray config as the **first stdout line** (`type: "vorn:config"`). Rust reads this before building the window.
- In full mode, `@vorn/core` redirects all `console.*` to **stderr** so stdout stays clean for IPC.
- Backend entry path is passed via `VORN_ENTRY` env var; frontend dir via `VORN_FRONTEND_DIR`; dev URL via `VORN_DEV_URL`.

### IPC topology (lite mode)

QuickJS can't read env vars and runs in-process, so:
- Window/frontend config is written to `globalThis.__vorn_config__` (a JSON string) and Rust reads it after eval.
- Events emit via `globalThis.__vorn_emit__(name, payloadJson)` injected by Rust.
- Frontend location is passed as **CLI args** (`--dev-url` or `--frontend-dir`) instead of env vars.
- Backend bundle is pre-built by the CLI to `.vorn/cache/bundle.dev.js` (dev) / `bundle.js` (build).

### Runtime detection — DCE trick

`@vorn/core` uses `globalThis.__VORN_RUNTIME__` to choose `_startFull` vs `_startLite`. The CLI's bundler replaces this at build time with a literal string (`define: { "globalThis.__VORN_RUNTIME__": '"lite"' }`), so Bun's dead-code elimination removes the unused branch entirely from each per-runtime bundle. **Don't introduce dynamic checks that defeat this.**

Runtime fallback: if the define wasn't applied (e.g. running unbundled), `__vorn_lite__` (set by the QuickJS host) is used instead.

### OS APIs — frontend → Rust direct

`dialog`, `vornWindow`, `clipboard`, `shell`, `notification`, `tray` calls from the frontend bypass the TS backend. Frontend posts `{ type: "os_call", api, method, args }` and Rust dispatches via `host-rs/src/os/mod.rs::dispatch`. `window` calls run on the main thread (event loop proxy); other OS calls run on a **fresh `std::thread`** per call (not rayon — dialogs block on user input and would starve a CPU pool).

### `vorn build` packaging — VORNPKG format

Both `vorn-full` and `vorn-lite` binaries can be self-extracting: `vorn build` appends a packed section to the host binary:

```
[file_count: u32 LE]
per file: [path_len: u16 LE][path: UTF-8][data_len: u32 LE][data: bytes]
[section_size: u64 LE]
[magic: "VORNPKG\0"]   ← 8 bytes, last bytes of binary
```

- Text files (`html|js|mjs|cjs|css|json|svg`) are auto-gzipped with `.gz` suffix appended to `rel`.
- `bundle.js` is **never auto-gzipped** (QuickJS reads it directly).
- Full mode also packs `bun.version` (must be first — Rust reads it before `bun.gz` to choose cache path) and the local `Bun.version` binary as `bun.gz` (gzip level 9).
- On Windows: `patchPeSubsystem` flips PE subsystem from console to GUI, then `setWindowsExeIcon` embeds icon resources into the final `.exe`. For full mode, the icon is also embedded into the **inner Bun copy** before gzipping so Task Manager shows the app icon for the subprocess.

Implementation in `packages/cli/src/commands/build.ts`; reader on the Rust side is `packages/{full,lite}/src/embed.rs`.

### Build cache

`vorn dev` and `vorn build` cache by hashing source dirs + key config files into `.vorn/cache/`. Three cache keys: `frontend`, `backend` (build), `backend-dev` (dev). When source hash matches and output exists, the step is skipped (`packages/cli/src/lib/cache.ts`).

### Frontend serving

Static frontend is served via `wry` custom protocol `bv://localhost/` → filesystem (`host-rs/src/scheme.rs`). `window.location.origin` is `bv://localhost`. The asset cache is **pre-warmed** on a background thread before the WebView is built, so the first request is instant.

### Lifecycle / shutdown

- `app.onReady` fires on `__vorn_page_ready` (a one-shot `postMessage` sent from `JS_PAGE_READY` init script on `DOMContentLoaded`).
- `app.onClose` fires on `WindowEvent::CloseRequested`. The window is hidden immediately; backend has **2 seconds** to run handlers before a watchdog thread sends `ForceExit`.

### SSR frameworks are blocked

`vorn dev`/`build` fast-fail if Next, Nuxt, SvelteKit, Remix, SolidStart, Angular Universal etc. are detected (`detect.ts:SERVER_FRAMEWORKS`). Vorn requires a pure SPA — it owns the server.

## CI

`.github/workflows/ci.yml` runs on push/PR to `main`:
- **Rust job** — matrix on `ubuntu-latest` / `windows-latest` / `macos-latest`: `cargo fmt --check` (Linux only), `cargo clippy -D warnings`, `cargo check --all-targets`.
- **TypeScript job** — `bunx biome ci`, `bun run typecheck`, and `bun run build` in `packages/cli`.

Run the local equivalents before pushing so CI stays green.

## Commits

Releases are cut by **release-please** from commit messages, so format matters:

- Use **Conventional Commits**: `feat:`, `fix:`, `chore:`, `docs:`, `refactor:`, `perf:`, `ci:`, `build:`, `test:`.
- Only `feat:` and `fix:` trigger version bumps. `feat!:` or `BREAKING CHANGE:` footer → major bump.
- Scope by component when useful: `feat(cli): ...`, `fix(core): ...`, `fix(host): ...` — matches the `cli`/`core`/`host` release-please components.
- One logical change per commit. Don't squash unrelated fixes together — release notes read straight from commit subjects.

## Releases

`release-please` manages releases for the **three npm packages** (`@vorn/cli`, `@vorn/core`, `@vorn/host`) in a single bundled PR (`separate-pull-requests: false` in `release-please-config.json`). Components: `cli`, `core`, `host` — tag format is `<component>-v<version>` (e.g. `host-v0.1.0`). Per-package changelogs live at `packages/<pkg>/CHANGELOG.md`.

The Rust binaries (`vorn-full`, `vorn-lite`) are built by `.github/workflows/build-host.yml` and uploaded as release assets to the `host-v*` tag. `@vorn/host`'s `postinstall` (`packages/host/install.ts`) downloads the matching `vorn-{runtime}-{platform}-{arch}[.exe]` for the host's platform. The postinstall **skips** when run from inside the workspace (path doesn't contain `node_modules`).
