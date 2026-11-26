# 🤝 Contributing to Tynd

Thanks for helping! This guide covers the dev loop, code style, documentation expectations, commit conventions, and the PR process.

## Licensing

By opening a pull request, you agree that your contribution is licensed under the [Apache License 2.0](./LICENSE) — same license as the rest of the project. No separate contributor agreement is needed.

---

## Setup

Use the repository versions in `.bun-version`, `.node-version`, and `rust-toolchain.toml` for local development. Node.js runs the documentation tooling and npm release steps. Bun runs the application backend and the TypeScript tests.

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
  libxdo-dev libxss-dev libpipewire-0.3-dev \
  libgbm-dev libdrm-dev libclang-dev libxcb1-dev libxrandr-dev \
  libdbus-1-dev libwayland-dev libegl-dev
```

Install Rust (1.95 or newer, via [rustup](https://rustup.rs)) for repository development and for any Tynd app: every app's host is compiled with `cargo`, and the native CLI does not download/manage a toolchain. Bun is required for application TypeScript, project commands, config, and package workflows.

---

## Repository layout

Hybrid Bun + Cargo monorepo:

| Directory | What |
|---|---|
| `crates/host` | Rust lib (`tynd-host`) — wry event loop, IPC, all OS APIs |
| `crates/runtime` | Rust bin (`tynd-runtime`) — spawns Bun application backend |
| `crates/cli` | Native Rust `tynd` + `tynd-mcp` command/protocol implementation |
| `crates/plugin-sdk`, `crates/plugin-macros` | Rust plugin SDK (`tynd-plugin-sdk`) and its proc macros (`tynd-plugin-macros`) |
| `crates/bundler`, `crates/bundle-contract` | Rust bin (`tynd-bundler`) — installers, signing, packs — and the request types the CLI sends it |
| `crates/*` | Shared Rust-only libraries and protocol crates |
| `packages/host-src` | `@tynd/host-src` npm — generated Rust source mirror for app-specific host builds |
| `packages/core` | `@tynd/core` — backend + `/client` frontend APIs |
| `packages/cli` | `@tynd/cli` — tiny native launcher, TypeScript config facade, schema/templates |
| `npm/tooling*` | `@tynd/tooling` resolver plus six `@tynd/tooling-<os>-<arch>` binary-carrier packages |
| `packages/plugin-sdk` | `@tynd/plugin-sdk` — TypeScript helpers; ships a generated mirror of the Rust SDK crates |
| `packages/mcp` | `@tynd/mcp` — tiny launcher plus static MCP catalog; server runs in `crates/cli` |
| `packages/test` | `@tynd/test` — end-to-end test harness driving an app over the debug protocol |
| `plugins/host` | Host-backed official plugins (`tynd.kind: "host"`): thin packages exposing host APIs |
| `plugins/standalone` | Standalone official plugins (`tynd.kind: "standalone"`): own native crate or pure Bun |
| `plugins/catalog.json` | Catalog of official plugins (`tynd add` names, aliases, packages) |
| `scripts` | Workspace generators and checks (API catalog, MCP catalog, release checks) |
| `apps/docs` | Documentation site |

### Shared configuration

The workspace owns the baseline config:

- TypeScript packages extend [`tsconfig.base.json`](./tsconfig.base.json).
- Biome packages extend [`biome.json`](./biome.json).
- Rust crates use root [`rust-toolchain.toml`](./rust-toolchain.toml), [`rustfmt.toml`](./rustfmt.toml), and [`clippy.toml`](./clippy.toml).

Keep package-level config only when it changes behavior for that package. Current examples are package `tsconfig.json` path aliases, package `biome.json` include/exclude lists, and the plugin SDK `clippy.toml` files that enable published-API linting.

---

## Dev loop

Every non-trivial change follows this loop:

1. **Design** — restate the goal, note TS / Rust / both, note IPC / packaging / cache impacts.
2. **Develop** — smallest surface that solves the problem; reuse existing helpers, don't add abstractions for hypothetical needs.
3. **Verify** — run the checks below until clean.
4. **Review the diff** — `git status` + `git diff`; read every hunk; remove dead code, stray prints.
5. **Build** — rebuild affected Rust tooling/host/bundler binaries or run `tynd build` against a playground.
6. **Document** — update docs in the **same commit** as the code (see "Docs" below).

### Verify commands

```bash
bun run check         # Biome, generated-file drift, tsc, docs typecheck, cargo check, `bun run test`
bun run test:plugins  # plugin tests (`bun test plugins`)
bun run fix           # apply Biome and Rust formatting fixes
```

CI also runs these; run the ones your change touches:

```bash
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo clippy --locked -p tynd-host --features api-all --all-targets -- -D warnings
cargo test --locked --workspace --features tynd-host/api-all
cargo deny --locked check  # advisories, licenses, bans, sources (deny.toml)
actionlint                 # after editing .github/workflows
```

### Regenerate derived files

Never edit generated files by hand. `bun run check` fails when the API or MCP catalog drifts; CI also fails when the `host-src` mirror drifts:

```bash
bun scripts/gen-api-catalog.ts               # API catalog, config schema and config types (from crates/host/Cargo.toml, plugins/catalog.json, crates/cli/src/config/model.rs)
bun run gen:mcp-catalog                      # packages/mcp/catalog/ (see "MCP" below)
bun run sync:host-src                        # packages/host-src/native/ (see "host-src mirror" below)
bun run --cwd packages/plugin-sdk sync       # packages/plugin-sdk/native/ and native-macros/ (gitignored)
cd apps/docs && bun run scripts/build-llms-txt.ts  # llms*.txt under apps/docs/public/
```

### Build commands

```bash
cargo build -p tynd-cli --bins
cargo build -p tynd-bundler
cargo build -p tynd-runtime --release
```

The CLI/MCP implementation lives in `crates/cli`; `packages/cli` and `packages/mcp` only validate their launchers/types. Build native tooling with Cargo. Bun package builds check the JavaScript facades and TypeScript config surface.

### Validate against a sample app

Exercise GUI behavior (windows, menus, tray, dialogs, and WebView rendering) by hand in a sample app. Scaffold one with the CLI:

```bash
bunx @tynd/cli create sample-app
cd sample-app && bun run dev
```

---

## Docs — update in the same commit

Docs drift silently. After any user-visible feature/fix/behavior change, update
the relevant files under `apps/docs` **in the same commit** as the code:

| File | Update when |
|---|---|
| [`README.md`](./README.md) | Pitch, quickstart, API summary table, or headline comparison changed. |
| `packages/<pkg>/README.md` | That package's public API surface changed. |
| [`NOTICE`](./NOTICE) | Added or removed a significant third-party dep. |
| Docs site — [API reference](https://kvnpetit.github.io/tynd/docs/v1.0/api) | Public OS API, backend API, or frontend RPC surface changed. |
| Docs site — [runtime](https://kvnpetit.github.io/tynd/docs/v1.0/concepts/runtimes) | Bun or host behavior changed. |
| Docs site — [frontend frameworks](https://kvnpetit.github.io/tynd/docs/v1.0/guides/frontend-frameworks) | Frontend framework support / detection / scaffolding changed. |
| Docs site — [getting started](https://kvnpetit.github.io/tynd/docs/v1.0/getting-started) | Quickstart flow, CLI command names, first-app sample changed. |
| Docs site — [troubleshooting](https://kvnpetit.github.io/tynd/docs/v1.0/troubleshooting) | New user-facing failure mode or error message. |
| Docs site — [code signing](https://kvnpetit.github.io/tynd/docs/v1.0/guides/code-signing) | Signing / notarization workflow changed. |

Doc-only PRs skip the build step but never skip this table.

---

## Code style

- **Keep it simple.** No abstractions for hypothetical future needs.
- **Comments earn their place.** Default to none. Write only when the *why* is non-obvious. Don't narrate *what* the code does.
- **No rot-prone comments.** No "added for X", "see issue #123", "used by Y". Those go in commit messages / PR descriptions.
- **No speculative error handling.** Validate at system boundaries only. Trust internal code.
- **Grep before adding a helper.** Check `packages/core`, `crates/host/src`, and `crates/cli/src` when CLI work is involved for an existing equivalent.
- **File-size discipline.** Crossing ~300 LOC is a smell. Split by concern.
- **Application TypeScript stays Bun-first.** Native CLI/MCP/build orchestration belongs in Rust. Tiny npm launchers may use portable JavaScript/Node primitives needed to locate and spawn platform binaries.

## Module and crate boundaries

Tynd already has internal Rust crate boundaries. Keep dependency direction explicit:

| Crate / package | Owns | May depend on |
|---|---|---|
| Rust leaf crates (`host-macros`, `capabilities`, `debug-protocol`, `pack-format`, `protocol`, `security-policy`, `update-core`) | narrow shared contracts | no other Tynd workspace crate |
| `crates/host` | event loop, WebView, IPC, OS APIs, plugin bridge | Rust leaf crates + native plugin SDK |
| `crates/runtime` | Bun-backed executable | `host`, `pack-format`, `protocol` |
| `crates/plugin-sdk` | public native plugin contract | `plugin-macros` |
| `crates/plugin-macros` | plugin proc macros | native SDK as a dev-only compile-fixture dependency |
| `plugins/standalone/*/native` | optional native workloads | native plugin SDK only |
| `packages/core` | public TypeScript runtime/client API | no other Tynd workspace package |
| `packages/host-src` | generated source mirror for app-specific hosts | generated from `crates/host`; edit source first |
| `crates/cli` | native CLI/MCP orchestration, config resolution, diagnostics, plugins, releases | narrow Rust dependencies + `@tynd/host-src` inputs for app hosts |
| `packages/cli` | launcher, dependency-free config types/schema/templates | `@tynd/tooling`, `@tynd/host-src` |
| `npm/tooling*` | resolve/distribute three native developer executables | six optional platform packages only |
| `packages/mcp` | MCP launcher and bounded static catalog | `@tynd/tooling` |
| `packages/plugin-sdk` | public TypeScript/native plugin contracts | `@tynd/core` as a peer |
| `plugins/host/*` | optional typed facades over host-native APIs | `@tynd/core` as a peer; no native implementation |
| `plugins/standalone/*` | optional Bun or native workloads | `@tynd/core` and, when needed, `@tynd/plugin-sdk` as peers |
| `apps/docs` | documentation application | no Tynd runtime workspace dependency |

`plugins/catalog.json` is the canonical first-party catalog. The CLI embeds it
at bundle time; packages must not import source from sibling workspaces. Keep
these dependency rules intact when moving code across workspace boundaries.

Create a new internal crate only when at least two of these are true:

- The code is used by more than one Rust binary/crate today, not "maybe later".
- The crate can expose a narrow, documented API with a stable ownership boundary.
- Splitting reduces compile time or feature-flag pressure for common builds.
- The code has a different safety or dependency profile than `crates/host` (for example crypto/signing, updater patching, or IPC framing).
- The split does not make `packages/host-src` synchronization more fragile.

Do not create a crate just because a file is large. First split a large module into private submodules under the same crate. Promote it to a crate after the boundary proves stable.

Current likely extraction candidates, if they continue growing:

- `host-security`: live policy, persisted grants, process/shell gates.
- `host-updater`: manifest parsing, patch/full download, signature verification, install strategy.
- `host-ipc`: JSON/binary framing and streaming protocol shared by host, runtime, and future tooling.

Each extraction must document its dependency direction and regenerate the `host-src` source mirror.

### Banned characters

No smart quotes (`"` `"` `'` `'`), no Unicode arrows (`→`, `←`, `⇒`). Use ASCII (`->`, `<-`, `=>`). Em-dash `—` is fine as prose punctuation; banned as a `:`/`-` replacement in tables or headers.

---

## Commit style

Conventional Commits — used to structure the changelog at release time:

| Prefix | Meaning |
|---|---|
| `feat:` | new feature (minor version bump) |
| `fix:` | bug fix (patch version bump) |
| `feat!:` or `BREAKING CHANGE:` footer | major version bump |
| `chore:`, `docs:`, `refactor:`, `perf:`, `ci:`, `build:` | no version change |

Scope when useful: `feat(cli): …`, `fix(core): …`, `fix(host): …`.

**One logical change per commit.** Don't squash unrelated fixes — release notes are cut straight from subjects.

**Never push without explicit authorization.** Local commits are fine; `git push` / `--force` / tag push / PR creation / release publish all require a direct ask.

---

## Pull requests

1. Fork and branch from `main`.
2. Run `bun run check` locally before opening.
3. Open the PR against `main`. CI runs the workspace check on Ubuntu.
4. Ensure the [Docs update table](#docs--update-in-the-same-commit) was followed.
5. Explicitly flag any runtime-heavy API changes (`terminal`, `workers`, `http`, `sidecar`, PTY streams) in the PR description so reviewers exercise them by hand.
6. One maintainer approval before merge.

### What reviewers check

- Diff is minimal and focused — no unrelated cleanups mixed in.
- Docs updated in the same commit.
- No banned characters (smart quotes, Unicode arrows).
- No speculative error handling or dead abstraction layers.
- Bun-first primitives used when available.

---

## `host-src` mirror

`packages/host-src/native/` is a generated mirror of `crates/host/`, `crates/runtime/` and the Rust crates they depend on (under `native/vendor/`), published as `@tynd/host-src` for out-of-workspace app-specific host builds. Edits go to the crates; the mirror is regenerated from there and committed (CI fails on drift).

```bash
bun run sync:host-src  # regenerate packages/host-src/native/ from crates/
```

The sync script (`packages/host-src/scripts/sync.ts`) flattens `workspace = true` references to concrete versions read from the root `Cargo.toml`, since the published crate has no workspace ancestor.

---

## Area notes

### CLI (`packages/cli`, `crates/cli`, `crates/bundler`)

Command behavior lives in Rust; `packages/cli/bin/index.js` is a platform launcher only and carries no command logic. `packages/cli/src/lib/config.ts` holds the dependency-free `defineConfig` facade; its types (`src/lib/config/types.generated.ts`) and `schemas/tynd.config.json` are generated from `crates/cli/src/config/model.rs` by `bun scripts/gen-api-catalog.ts`.

| Area | Owns |
|---|---|
| `crates/cli/src/bin/tynd.rs` | native CLI entry point |
| `crates/cli/src/args.rs` | public Clap commands and arguments |
| `crates/cli/src/commands/` | project, plugin, release, test and tooling commands |
| `crates/cli/src/project/` | project discovery, custom-host resolution, platform binaries |
| `crates/bundler/src` | installers, signing, patches, packs (`tynd-bundler`) |
| `crates/bundle-contract/src` | JSON requests the CLI sends to `tynd-bundler` |

Update `packages/cli/README.md` whenever a public command or flag changes.

```bash
cargo check -p tynd-cli --bins
cargo check -p tynd-bundler
```

### MCP (`packages/mcp`, `crates/cli/src/mcp`)

The server is native Rust (`crates/cli/src/bin/tynd-mcp.rs` plus `crates/cli/src/mcp/`). Do not add a parallel TypeScript implementation. The npm package ships only `bin/index.js`; `crates/cli/build.rs` compiles the static catalog into the binary.

```bash
cargo run -p tynd-cli --bin tynd-mcp -- serve
cargo check -p tynd-cli --bin tynd-mcp
cargo test -p tynd-cli mcp::catalog  # validates the embedded catalog
bun run gen:mcp-catalog    # regenerate packages/mcp/catalog/ from its sources
bun run check:mcp-catalog  # fail when the catalog drifted (part of `bun run check`)
```

The catalog is generated by `scripts/gen-mcp-catalog.ts`; never edit it by hand. `framework-docs/*.md` come from the v1.0 docs pages (`apps/docs/src/content/docs/docs/v1.0/`, MDX converted to markdown) and `plugin-schemas/*.gen.ts` from `plugins/standalone/<name>/<name>.gen.ts`. After editing a mapped docs page or regenerating a plugin schema, run `bun run gen:mcp-catalog` and commit the result.

Catalog rules (`packages/mcp/catalog/`), enforced by the `mcp::catalog` tests:

- `framework-docs/*.md` filenames are resource slugs; keep them stable. Each needs YAML frontmatter with `title` and `description`, and a level-one heading.
- `plugin-schemas/*.gen.ts` must contain an exported TypeScript declaration.
- Each asset must be non-empty and at most 2 MiB.
- Adding or removing an asset means updating `FRAMEWORK_DOCS` / `PLUGIN_SCHEMAS` in `scripts/gen-mcp-catalog.ts`, then regenerating; `build.rs` embeds whatever it generated.

Update `packages/mcp/README.md` whenever a public command, flag, tool, resource, prompt, or security default changes.

### Plugin SDK (`packages/plugin-sdk`, `crates/plugin-sdk`, `crates/plugin-macros`)

Two surfaces: the `@tynd/plugin-sdk` TypeScript helpers and the `tynd-plugin-sdk` Rust crate (`crates/plugin-sdk`, with proc macros in `crates/plugin-macros`). The SDK is the contract between hosts and third-party plugins, so it is semver-bound:

- Breaking changes to the `TyndPlugin` trait, the event wire format, or the `definePlugin` shape are major (`feat!:`); call them out explicitly in the PR.
- Additive helpers or types are minor; fixes are patch.
- The npm package and the Rust crate versions move in lockstep.

Public APIs need `///` doc comments (Rust) and JSDoc (TypeScript). `packages/plugin-sdk/docs/` is generated output and gitignored; never commit it. The npm tarball ships the Rust crates as `native/` and `native-macros/`: standalone mirrors generated at `prepack` by `bun run --cwd packages/plugin-sdk sync` and gitignored; edit the crates, never the mirrors.

```bash
bun run --cwd packages/plugin-sdk build
cargo clippy -p tynd-plugin-sdk -- -D warnings
```

### Standalone plugins (`plugins/standalone`)

Each plugin is `<plugin>/` (npm `@tynd/plugin-<plugin>`) plus, for native plugins, `<plugin>/native/` (Rust crate `tynd-plugin-<plugin>`, a member of the root Cargo workspace). Bun/TypeScript-only plugins such as `sql` have no `native/`.

```bash
cargo build --release -p tynd-plugin-terminal  # one native plugin
```

- A native crate depends only on the plugin SDK; shared workspace dependencies are version pins, not shared logic. Extract a shared helper crate only when the same logic is duplicated in at least three plugins.
- Put heavy optional backends behind features (for example, `capture` for audio) so default builds stay portable and fast.
- Touch one plugin per PR when possible, and update that plugin's `README.md` and `CHANGELOG.md` for public API or behavior changes.

### Docs site (`apps/docs`)

An Astro + Starlight site published to GitHub Pages; content is MDX under `apps/docs/src/content/docs/docs`. `prebuild` regenerates the `llms*.txt` files under `public/` (gitignored).

```bash
cd apps/docs
bun run dev          # local server
bun run typecheck    # astro check + scripts
bun run build        # static site and Pagefind index in ./dist
bun run check:links  # after a build: verify internal links
```

Content style: English only, active voice, code samples that run as written, valid cross-links and anchors, compressed screenshots, and no formatting churn in content-only PRs.

---

## Releases

Maintainers publish releases manually from GitHub Actions.

1. Update the package versions, exact tooling pins, Cargo versions, lockfiles and changelog for the scope being released.
2. Push the release commit to `main` and wait for CI on that exact commit.
3. Create and push `vX.Y.Z` for the framework or `plugins-vX.Y.Z` for standalone plugins. Run the corresponding workflow and supply the tag.
4. For a framework release, select the tag as the workflow ref, or select `main` while it still points to the tagged commit. Approve publication in the `release` environment. The workflow uses the configured `NPM_TOKEN` to publish public packages with provenance.

Release workflows build the three native developer executables for macOS, Linux, and Windows on x64/ARM64. No prebuilt application host is published: every app compiles its own host with cargo from `@tynd/host-src`. Published npm packages include `@tynd/tooling` plus six platform packages, `@tynd/cli`, `@tynd/mcp`, `@tynd/core`, `@tynd/host-src`, and plugin SDK/packages in release scope.

CLI, MCP and tooling versions move together. The release stages `tynd`, `tynd-mcp` and `tynd-bundler` into the six `@tynd/tooling-<os>-<arch>` packages. `@tynd/mcp` ships with the framework version and an exact `@tynd/tooling` pin. `scripts/check-release-version.ts` rejects versions and pins that differ from the tag.

Changes to the framework release build configuration on `main` trigger **Release validation**, which compiles all six native targets and verifies the bundler assets and attestations without publishing to npm or GitHub Releases. The manual framework workflow uses the same build and verifies the remote tag again before publishing. Enable `verify_reproducibility` for a clean rebuild and digest comparison before publication.

Both Linux tooling targets use Ubuntu 24.04 runners, with a native ARM64 runner for Linux ARM64. Standalone plugin validation also uses Ubuntu 24.04 and the shared CI system dependencies, including the ALSA headers required by the audio plugin's published features.

Contributors don't touch versions — only maintainers cut releases.

---

## Questions?

Open an [issue](https://github.com/kvnpetit/tynd/issues), a [GitHub Discussion](https://github.com/kvnpetit/tynd/discussions), or a draft PR. For architectural context, the [docs site concepts section](https://kvnpetit.github.io/tynd/docs/v1.0/concepts/architecture) covers IPC, the TYNDPKG format, generated staging, and Bun runtime.
