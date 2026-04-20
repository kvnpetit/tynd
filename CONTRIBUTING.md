# 🤝 Contributing to Tynd

Thanks for helping! This guide covers the dev loop, code style, test & doc expectations, commit conventions, and the PR process.

## Licensing

By opening a pull request, you agree that your contribution is licensed under the [Apache License 2.0](./LICENSE) — same license as the rest of the project. No separate contributor agreement is needed.

---

## Setup

```bash
git clone https://github.com/kvnpetit/tynd
cd tynd
bun install
```

Linux system deps (match CI):

```bash
sudo apt-get install -y \
  libgtk-3-dev libwebkit2gtk-4.1-dev \
  libjavascriptcoregtk-4.1-dev libsoup-3.0-dev \
  libxdo-dev
```

The Rust toolchain auto-installs on first `cargo` run (pinned via `rust-toolchain.toml`). Bun is required — install once from [bun.sh](https://bun.sh/).

---

## Repository layout

Hybrid Bun + Cargo monorepo. See [`CLAUDE.md`](./CLAUDE.md) for the full architecture tour, but the short version:

| Directory | What |
|---|---|
| `packages/host-rs` | Rust lib (`tynd-host`) — wry event loop, IPC, all OS APIs |
| `packages/full` | Rust bin (`tynd-full`) — spawns Bun subprocess |
| `packages/lite` | Rust bin (`tynd-lite`) — embeds QuickJS (~6.5 MB) |
| `packages/host` | `@tynd/host` npm — postinstall downloads pre-built binaries |
| `packages/core` | `@tynd/core` — backend + `/client` frontend APIs |
| `packages/cli` | `@tynd/cli` — `tynd create/dev/start/build/init/...` |
| `playground/*` | Hand-testing apps (`lite`, `full`, per-framework smoke) |

---

## Dev loop

Every non-trivial change follows this loop:

1. **Design** — restate the goal, note TS / Rust / both, note IPC / packaging / cache impacts.
2. **Develop** — smallest surface that solves the problem; reuse existing helpers, don't add abstractions for hypothetical needs.
3. **Verify** — run the checks below until clean.
4. **Review the diff** — `git status` + `git diff`; read every hunk; remove dead code, stray prints.
5. **Build** — rebuild affected artifacts (CLI bundle, Rust binary, or a `tynd build` against a playground).
6. **Document** — update docs in the **same commit** as the code (see "Docs" below).

### Verify commands

```bash
bun run check:all            # biome + typecheck + rustfmt + clippy + cargo check + bun test + cargo test
bun run check                # biome auto-fix (run before commit)
bun run fix:all              # biome + rustfmt auto-fix both sides

# Individual gates
bun run typecheck            # tsc --noEmit across workspaces
bun run check:ts             # biome ci + typecheck
bun run check:rust           # cargo fmt --check + clippy + cargo check
bun run test                 # bun test + cargo test
```

### Build commands

```bash
cargo build -p tynd-full --release
cargo build -p tynd-lite --release
bun run --cwd packages/cli build         # bundles CLI to packages/cli/bin/index.js
```

### Test against a playground

```bash
bun run --cwd playground/example build
./playground/example/release/example.exe
```

GUI behavior (windows, menus, tray, dialogs, WebView rendering) must be exercised by hand in a `playground/` app — unit tests don't cover that.

---

## Tests — what to cover, what to skip

**Cover** with a unit test in the same commit as the feature:

- IPC core (`createBackend`, on/off/once, stream protocol)
- Wire formats (TYNDPKG packing/unpacking, base64 helpers)
- Cache hashing (`.tynd/cache/` keys)
- OS APIs doing real work (`fs`, `store`, `process`, `compute`, `sql`)

**Skip** (exercise by hand in a playground instead):

- GUI-interactive APIs (`dialog`, `tyndWindow`, `tray`)
- Long-running streams (`terminal`, `workers` inside lite)
- Network-dependent APIs (`http` against real servers, `websocket`)
- Thin wrappers around Rust std (`os_info`, most of `path`)

**Pattern**:

- TS: next to the module as `<name>.test.ts`, `import { describe, test, expect } from "bun:test"`.
- Rust: inline `#[cfg(test)] mod tests { … }` at the bottom of the module.
- Tests touching the filesystem or global statics must generate a unique namespace / `mkdtemp` dir per test. Don't rely on test ordering.

A `feat:` commit without tests on a covered surface is a review smell.

---

## Docs — update in the same commit

Docs drift silently. After any user-visible feature/fix/behavior change, touch the relevant file(s) below **in the same commit** as the code:

| File | Update when |
|---|---|
| [`COMPARISON.md`](./COMPARISON.md) | **Every feature** touching a tracked category — flip ❌/⚠️ to ✅ and bump `Last updated`. |
| [`API.md`](./API.md) | Public OS API, backend API, or frontend RPC surface changed. |
| [`README.md`](./README.md) | Pitch, quickstart, API summary table, or headline comparison changed. |
| [`RUNTIMES.md`](./RUNTIMES.md) | Feature lands in only one of `full`/`lite`, or parity shifted. |
| [`ALTERNATIVES.md`](./ALTERNATIVES.md) | New lite gap with a known pure-JS fill, or a gap closed natively. |
| [`FRAMEWORKS.md`](./FRAMEWORKS.md) | Frontend framework support / detection / scaffolding changed. |
| [`GETTING_STARTED.md`](./GETTING_STARTED.md) | Quickstart flow, CLI command names, first-app sample changed. |
| [`TROUBLESHOOTING.md`](./TROUBLESHOOTING.md) | New user-facing failure mode or error message. |
| [`SIGNING.md`](./SIGNING.md) | Signing / notarization workflow changed. |
| `packages/<pkg>/README.md` | That package's public API or CLI surface changed. |
| [`NOTICE`](./NOTICE) | Added or removed a significant third-party dep. |
| [`CLAUDE.md`](./CLAUDE.md) | Architecture, IPC, cache keys, packaging format, workflow changed. |

Doc-only PRs skip the build step but never skip this table.

---

## Code style

- **Keep it simple.** No abstractions for hypothetical future needs.
- **Comments earn their place.** Default to none. Write only when the *why* is non-obvious. Don't narrate *what* the code does.
- **No rot-prone comments.** No "added for X", "see issue #123", "used by Y". Those go in commit messages / PR descriptions.
- **No speculative error handling.** Validate at system boundaries only. Trust internal code.
- **Grep before adding a helper.** Check `packages/core`, `packages/cli/src/lib`, `packages/host-rs/src` for an existing equivalent.
- **File-size discipline.** Crossing ~300 LOC is a smell. Split by concern.
- **Bun-first.** Prefer `Bun.file`, `Bun.write`, `Bun.spawn`, `Bun.hash.wyhash`. `node:*` only where no Bun equivalent exists (`node:path`, sync `node:fs` primitives, `node:os`, `node:util`, `node:zlib`).

### Banned characters

No smart quotes (`"` `"` `'` `'`), no Unicode arrows (`->`, `<-`, `|>`). Use ASCII (`->`, `<-`, `|>`). Em-dash `—` is fine as prose punctuation; banned as a `:`/`-` replacement in tables or headers.

---

## Commit style

Conventional Commits — release-please drives the changelog and version bumps from them:

| Prefix | Effect |
|---|---|
| `feat:` | new feature — minor bump |
| `fix:` | bug fix — patch bump |
| `feat!:` or `BREAKING CHANGE:` footer | major bump |
| `chore:`, `docs:`, `refactor:`, `perf:`, `ci:`, `build:`, `test:` | no version bump |

Scope when useful: `feat(cli): …`, `fix(core): …`, `fix(host): …`. Components map to release-please: `cli`, `core`, `host`.

**One logical change per commit.** Don't squash unrelated fixes — release notes are cut straight from subjects.

**Never push without explicit authorization.** Local commits are fine; `git push` / `--force` / tag push / PR creation / release publish all require a direct ask.

---

## Pull requests

1. Fork and branch from `main`.
2. Run `bun run check:all` locally — must pass before opening.
3. Open the PR against `main`. CI runs the same checks on Ubuntu, Windows, macOS.
4. Ensure the [Docs update table](#docs--update-in-the-same-commit) was followed.
5. Explicitly flag any runtime-heavy API changes (`terminal`, `workers`, `http`, `sidecar`, PTY streams) in the PR description so reviewers exercise them by hand.
6. One maintainer approval before merge.

### What reviewers check

- Diff is minimal and focused — no unrelated cleanups mixed in.
- Tests added for covered surfaces.
- Docs updated in the same commit.
- No banned characters (smart quotes, Unicode arrows).
- No speculative error handling or dead abstraction layers.
- Bun-first primitives used when available.

---

## Releases

Releases are cut by **release-please** from commit messages. Every merged `feat:` / `fix:` to `main` accumulates into a release PR maintained by the bot. Maintainers merge that PR to publish.

Rust binaries (`tynd-full`, `tynd-lite`) are built on the `v*` tag push by `.github/workflows/build-host.yml` across 6 architectures. `@tynd/cli`, `@tynd/core`, `@tynd/host` publish to npm in lockstep with the tag.

Contributors never touch versions by hand — release-please handles it.

---

## Questions?

Open an [issue](https://github.com/kvnpetit/tynd/issues) or a draft PR. For architectural questions, skim [`CLAUDE.md`](./CLAUDE.md) first — it covers the non-obvious bits of IPC, the TYNDPKG format, cache keys, and the `embedded-js` feature gate.
