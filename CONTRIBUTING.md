# 🤝 Contributing to Tynd

Thanks for helping! This guide covers the dev loop, code style, test & doc expectations, commit conventions, and the PR process.

## Licensing

By opening a pull request, you agree that your contribution is licensed under the [Apache License 2.0](./LICENSE) — same license as the rest of the project. No separate contributor agreement is needed.

---

## Setup

```bash
git clone https://github.com/tynd-apps/tynd
cd tynd
bun install
```

Linux system deps (match CI):

```bash
sudo apt-get install -y \
  libgtk-3-dev libwebkit2gtk-4.1-dev \
  libjavascriptcoregtk-4.1-dev libsoup-3.0-dev \
  libxdo-dev libxss-dev
```

The Rust toolchain auto-installs on first `cargo` run (pinned via `rust-toolchain.toml`). Bun is required — install once from [bun.sh](https://bun.sh/).

---

## Repository layout

Hybrid Bun + Cargo monorepo:

| Directory | What |
|---|---|
| `packages/host-rs` | Rust lib (`tynd-host`) — wry event loop, IPC, all OS APIs |
| `packages/full` | Rust bin (`tynd-full`) — spawns Bun subprocess |
| `packages/lite` | Rust bin (`tynd-lite`) — embeds QuickJS (~6.5 MB) |
| `packages/host` | `@tynd/host` npm — postinstall downloads pre-built binaries |
| `packages/host-src` | `@tynd/host-src` npm — Rust source consumed by the CLI when building custom runtimes |
| `packages/core` | `@tynd/core` — backend + `/client` frontend APIs |

Sibling repos (under [`tynd-apps`](https://github.com/tynd-apps)):

- [`tynd-apps/cli`](https://github.com/tynd-apps/cli) — `@tynd/cli` (`tynd create/dev/start/build/init/...`).
- [`tynd-apps/plugin-sdk`](https://github.com/tynd-apps/plugin-sdk) — `@tynd/plugin-sdk` + Rust SDK trait.
- [`tynd-apps/plugins`](https://github.com/tynd-apps/plugins) — official plugins (sql, terminal, qr, i18n, …).

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

### Test against a sample app

GUI behavior (windows, menus, tray, dialogs, WebView rendering) must be exercised by hand in a sample app — unit tests don't cover that. Scaffold one with the CLI from a sibling clone:

```bash
bunx @tynd/cli create sample-app --runtime full
cd sample-app && bun run dev
```

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

Docs drift silently. After any user-visible feature/fix/behavior change, touch the relevant file(s) below **in the same commit** as the code (or in a synchronized PR against the [`tynd-apps/docs`](https://github.com/tynd-apps/docs) repo when the change targets the docs site):

| File | Update when |
|---|---|
| [`README.md`](./README.md) | Pitch, quickstart, API summary table, or headline comparison changed. |
| [`COMPARISON.md`](./COMPARISON.md) | **Every feature** touching a tracked category — flip ❌/⚠️ to ✅ and bump `Last updated`. |
| `packages/<pkg>/README.md` | That package's public API surface changed. |
| [`NOTICE`](./NOTICE) | Added or removed a significant third-party dep. |
| Docs site — [API reference](https://tynd.kvnpetit.com/docs/api) | Public OS API, backend API, or frontend RPC surface changed. |
| Docs site — [runtimes](https://tynd.kvnpetit.com/docs/runtimes) | Feature lands in only one of `full`/`lite`, or parity shifted. |
| Docs site — [frontend frameworks](https://tynd.kvnpetit.com/docs/guides/frontend-frameworks) | Frontend framework support / detection / scaffolding changed. |
| Docs site — [getting started](https://tynd.kvnpetit.com/docs/getting-started) | Quickstart flow, CLI command names, first-app sample changed. |
| Docs site — [troubleshooting](https://tynd.kvnpetit.com/docs/troubleshooting) | New user-facing failure mode or error message. |
| Docs site — [code signing](https://tynd.kvnpetit.com/docs/guides/code-signing) | Signing / notarization workflow changed. |

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

Conventional Commits — used to structure the changelog at release time:

| Prefix | Meaning |
|---|---|
| `feat:` | new feature (minor version bump) |
| `fix:` | bug fix (patch version bump) |
| `feat!:` or `BREAKING CHANGE:` footer | major version bump |
| `chore:`, `docs:`, `refactor:`, `perf:`, `ci:`, `build:`, `test:` | no version change |

Scope when useful: `feat(cli): …`, `fix(core): …`, `fix(host): …`.

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

Releases are cut manually by maintainers. The flow:

1. Bump versions in `package.json` + `Cargo.toml` (workspace + members).
2. Update `CHANGELOG.md` with the new entry.
3. Commit `chore: release vX.Y.Z`, tag `vX.Y.Z`, push tag.

Rust binaries (`tynd-full`, `tynd-lite`) are built on the `v*` tag push by `.github/workflows/build-host.yml` across 6 architectures. `@tynd/core`, `@tynd/host`, `@tynd/host-src` publish to npm in lockstep with the tag. Sibling repos (`cli`, `plugin-sdk`, `plugins`) cut their own releases.

Contributors don't touch versions — only maintainers cut releases.

---

## Questions?

Open an [issue](https://github.com/tynd-apps/tynd/issues), a [GitHub Discussion](https://github.com/tynd-apps/tynd/discussions), or a draft PR. For architectural context, the [docs site concepts section](https://tynd.kvnpetit.com/docs/concepts) covers IPC, the TYNDPKG format, cache keys, and the runtime split.
