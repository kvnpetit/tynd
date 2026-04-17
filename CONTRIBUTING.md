# Contributing to Tynd

Thanks for helping!

## Licensing

By opening a pull request, you agree that your contribution is licensed
under the [Apache License 2.0](./LICENSE) — the same license as the rest
of the project. No separate contributor agreement is needed.

## Setup

```bash
git clone https://github.com/kvnpetit/tynd
cd tynd
bun install
```

Linux system deps:

```bash
sudo apt-get install -y \
  libgtk-3-dev libwebkit2gtk-4.1-dev \
  libjavascriptcoregtk-4.1-dev libsoup-3.0-dev
```

The Rust toolchain auto-installs on first `cargo` run (pinned via `rust-toolchain.toml`).

## Dev loop

```bash
bun run check:all            # biome + typecheck + rustfmt + clippy + cargo check
bun run check                # biome auto-fix

cargo build -p tynd-full --release
cargo build -p tynd-lite --release
bun run --cwd packages/cli build
```

Test against a playground:

```bash
bun run --cwd playground/full build
./playground/full/release/full.exe
```

## Commit style

Conventional Commits — release-please drives the changelog from them:

| Prefix | Effect |
|---|---|
| `feat:` | new feature — minor bump |
| `fix:` | bug fix — patch bump |
| `feat!:` or `BREAKING CHANGE:` footer | major bump |
| `chore:`, `docs:`, `refactor:`, `perf:`, `ci:`, `build:`, `test:` | no bump |

Scopes: `cli`, `core`, `host`, `full`, `lite`. Example:

```
feat(cli): add --json flag to info
```

## Pull requests

1. Fork + branch from `main`
2. `bun run check:all` must pass locally
3. Open the PR against `main` — CI runs the same checks
4. One maintainer approval before merge

## Questions?

Open an [issue](https://github.com/kvnpetit/tynd/issues) or a draft PR.
