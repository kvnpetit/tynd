# Official Plugins Audit and Remediation Implementation Plan

> **For Codex:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Prove correctness and maintainability for every official plugin package, fix every confirmed finding, and close all plugin coverage rows.

**Architecture:** Review each package as a JavaScript facade plus optional Rust resource owner. Use generated command bindings as the contract boundary, validate lifecycle cleanup and platform feature gates, and extract shared SDK logic only when at least two concrete implementations prove the same responsibility.

**Tech Stack:** Bun, TypeScript, Rust 1.94, Bun test, Cargo test/Clippy, snapshots, generated Specta bindings.

## Global Constraints

- Execute the gate-stabilization plan first so frozen install is trustworthy.
- Preserve public package names, exports, command names, event names, serialized shapes, and feature flags.
- Do not split large `lib.rs` files by line count alone.
- Native handles, threads, sockets, callbacks, and streams require explicit stop/drop evidence.
- No hardware-dependent test is treated as passing unless it ran or has an explicit platform proof gap.
- Every plugin file gets a closed coverage-ledger row.

---

### Task 1: Establish clean plugin gates and package inventory

**Files:**
- Modify: `..\plugins\TESTING.md` only if an executed command is missing from documented contributor gates.
- Modify: central coverage and finding ledgers.

**Interfaces:**
- Consumes: thirteen Bun workspaces and eleven native Cargo members.
- Produces: one baseline receipt per package and zero installation ambiguity.

- [ ] **Step 1: Run clean setup and static gates**

From `D:\Projects\tynd-workspace\plugins`:

    rtk proxy bun install --frozen-lockfile
    rtk proxy bun run check
    rtk proxy bun run deps
    rtk proxy bun run typecheck
    rtk proxy bun run typecheck:rust
    rtk proxy bun run lint:rust

Expected: all exit 0. The format check may remain red only because `cargo fmt --all` traverses the local `tynd_plugin_sdk` path dependency; that coupling is closed after the Tynd formatting task.

- [ ] **Step 2: Verify exact workspace membership**

    cargo metadata --no-deps --format-version 1

Confirm exactly these native crates:

```text
tynd_plugin_audio
tynd_plugin_http_server
tynd_plugin_keyring
tynd_plugin_llm
tynd_plugin_oauth
tynd_plugin_screen_rec
tynd_plugin_sql
tynd_plugin_terminal
tynd_plugin_tts
tynd_plugin_wgpu
tynd_plugin_whisper
```

Confirm Bun workspaces also include `command-palette` and `gpu`.

- [ ] **Step 3: Attach baseline evidence**

For each package, add `check`, `deps`, TypeScript/Rust check, and manifest review to its coverage rows. Keep source rows pending until Tasks 2–4 complete.

---

### Task 2: Audit pure TypeScript plugin behavior

**Files:**
- Inspect: `..\plugins\command-palette\client.ts`
- Inspect: `..\plugins\command-palette\client.test.ts`
- Inspect: `..\plugins\gpu\client.ts`
- Inspect: `..\plugins\gpu\benchmark.ts`
- Inspect: both package manifests and READMEs.
- Modify only if a confirmed finding receives a red test and exact plan addendum.

**Interfaces:**
- Consumes: browser keyboard events, command registry callbacks, WebGPU/WebGL capability probes.
- Produces: deterministic cleanup, ranking, error propagation, and feature fallback receipts.

- [ ] **Step 1: Run existing tests**

    rtk proxy bun test command-palette/client.test.ts
    rtk proxy bun test gpu
    rtk proxy bun run typecheck

- [ ] **Step 2: Review command-palette invariants**

Prove from full-file code and tests:

- duplicate command IDs have defined overwrite behavior;
- hidden commands never rank or run;
- failing listeners do not block other listeners;
- replaced shortcuts detach old handlers;
- `clear` and `unregister` release all references;
- async action failure is propagated and close behavior is deterministic.

Record exact test names/lines as evidence. A missing invariant becomes a candidate before any source edit.

- [ ] **Step 3: Review GPU invariants**

Prove:

- feature detection does not throw when WebGPU/WebGL globals are absent;
- benchmark buffers/devices are released;
- unsupported adapters produce a typed result rather than a crash;
- timeout or lost-device paths terminate;
- package exports match README examples.

If runtime proof needs a browser, record the exact unvalidated branch and defer only with user approval.

- [ ] **Step 4: Close pure-TS rows**

Mark each inspected source/test/manifest/doc row `clean`, `finding`, or explicit `deferred` with evidence. Run the plugin ledger closure query after updates.

---

### Task 3: Audit native dispatch and generated contracts for every plugin

**Files:**
- Inspect: `..\plugins\{audio,http-server,keyring,llm,oauth,screen-rec,sql,terminal,tts,wgpu,whisper}\index.ts`
- Inspect: every matching `*.gen.ts`.
- Inspect: every `native/src/lib.rs` and `native/src/bin/gen-types.rs`.
- Inspect: native tests, snapshots, benches, and soak tests.

**Interfaces:**
- Consumes: `TyndPlugin::dispatch`, generated command catalogs, plugin SDK context/events.
- Produces: one command-by-command contract receipt and generated-binding drift proof per native plugin.

- [ ] **Step 1: Run deterministic native tests package by package**

    rtk cargo test -p tynd_plugin_audio
    rtk cargo test -p tynd_plugin_http_server
    rtk cargo test -p tynd_plugin_keyring
    rtk cargo test -p tynd_plugin_llm
    rtk cargo test -p tynd_plugin_oauth
    rtk cargo test -p tynd_plugin_screen_rec
    rtk cargo test -p tynd_plugin_sql
    rtk cargo test -p tynd_plugin_terminal
    rtk cargo test -p tynd_plugin_tts
    rtk cargo test -p tynd_plugin_wgpu
    rtk cargo test -p tynd_plugin_whisper

Expected: deterministic unit/snapshot tests pass. Hardware-gated tests may report ignored/disabled status, which must be recorded precisely.

- [ ] **Step 2: Check every generated binding**

Run every existing native generator from `D:\Projects\tynd-workspace\plugins`:

    rtk cargo run --manifest-path audio/native/Cargo.toml --bin gen-types
    rtk cargo run --manifest-path http-server/native/Cargo.toml --bin gen-types
    rtk cargo run --manifest-path keyring/native/Cargo.toml --bin gen-types
    rtk cargo run --manifest-path llm/native/Cargo.toml --bin gen-types
    rtk cargo run --manifest-path oauth/native/Cargo.toml --bin gen-types
    rtk cargo run --manifest-path screen-rec/native/Cargo.toml --bin gen-types
    rtk cargo run --manifest-path sql/native/Cargo.toml --bin gen-types
    rtk cargo run --manifest-path terminal/native/Cargo.toml --bin gen-types
    rtk cargo run --manifest-path tts/native/Cargo.toml --bin gen-types
    rtk cargo run --manifest-path wgpu/native/Cargo.toml --bin gen-types
    rtk cargo run --manifest-path whisper/native/Cargo.toml --bin gen-types

Then run `rtk git diff -- '*.gen.ts'` and compare every generated binding byte-for-byte. Record any drift before deciding whether the generator or committed output is authoritative.

Repeat with `http-server`, `keyring`, `llm`, `oauth`, `screen-rec`, `sql`, `terminal`, `tts`, `wgpu`, and `whisper`.

- [ ] **Step 3: Review dispatch invariants**

For each command arm, record:

- validated argument type and size/range limits;
- resource/handle ownership;
- event names and payload schema;
- unknown-handle behavior;
- stop/cancel/drop cleanup;
- lock scope and poisoning strategy;
- platform/feature behavior;
- generated TypeScript method and return type.

Every `unwrap`, detached thread, global registry, unsafe block, or unbounded queue receives an evidence note.

- [ ] **Step 4: Validate facade invariants**

Read each TypeScript facade completely. Confirm:

- failed start promises surface to iterators/callers;
- iterator `return()` calls stop exactly once;
- subscriptions detach on stop and failed start;
- base64/binary conversion copies or borrows intentionally;
- event handlers filter by handle ID;
- default export and named export match package.json.

- [ ] **Step 5: Add exact remediation tasks before changing code**

For every confirmed candidate, append a numbered task to this plan containing exact source/test files, the failing test body, minimal implementation, and focused commands. Do not patch a candidate directly from the audit checklist.

---

### Task 4: Audit cross-plugin duplication and package boundaries

**Files:**
- Inspect: all native `lib.rs` files, plugin SDK helpers, root `Cargo.toml`, root `package.json`, and package manifests.
- Modify only through a reviewed addendum when extraction is proven.

**Interfaces:**
- Consumes: repeated registry, event, option-normalization, stop, and generated-type patterns.
- Produces: explicit `duplicate-justified` or finding dispositions; no speculative shared abstraction.

- [ ] **Step 1: Compare known large modules**

Review complete files:

- `audio/native/src/lib.rs`;
- `http-server/native/src/lib.rs`;
- `whisper/native/src/lib.rs`;
- `llm/native/src/lib.rs`;
- `screen-rec/native/src/lib.rs`.

Map responsibilities and internal state. A file is a refactor candidate only when two independently testable responsibilities can be named and their interface written.

- [ ] **Step 2: Search exact and structural duplicates**

    rtk grep "DashMap<" . --glob "*.rs"
    rtk grep "createHandleEvents" . --glob "*.ts"
    rtk grep "unknown_method" . --glob "*.rs"
    rtk grep "gen_types_bin" . --glob "*.rs"

Record whether duplication belongs in the plugin SDK, is platform-specific, or is intentionally package-local.

- [ ] **Step 3: Verify dependency direction**

    rtk cargo tree --workspace --edges normal
    rtk proxy bun run deps:all

Confirm plugins depend on the SDK and shared crates, never on another plugin implementation. Confirm the sibling SDK path dependency is deliberate and documented.

- [ ] **Step 4: Close duplication rows**

Use `duplicate-justified` only with exact reason and owner. Any extraction finding requires characterization tests in both source callers before implementation.

---

### Task 5: Run plugin coverage and lifecycle stress gates

**Files:**
- Verify existing Bun, Cargo, soak, and coverage artifacts.

- [ ] **Step 1: Run full deterministic gates**

    rtk proxy bun run verify:full
    rtk proxy bun run check:all
    rtk proxy bun run deps:all
    rtk proxy bun run coverage:ci
    rtk proxy bun run coverage:rust

- [ ] **Step 2: Run existing soak tests explicitly**

    rtk cargo test -p tynd_plugin_http_server --test load_soak -- --ignored
    rtk cargo test -p tynd_plugin_terminal --test pty_soak -- --ignored

Run `screen-rec`/audio/hardware tests only when required hardware is present; otherwise record exact ignored tests and environment gap.

- [ ] **Step 3: Verify Rust formatting after Tynd SDK formatting**

    rtk proxy bun run format:check:rust

Expected: exit 0. If it reports only sibling Tynd paths, the Tynd formatting task is incomplete; do not change plugin files to hide it.

---

### Task 6: Prove plugin ledger closure

**Files:**
- Modify: central `coverage-ledger.json` and `findings.json`.

- [ ] **Step 1: Re-run inventory and closure query**

    rtk proxy bun --cwd ..\tynd run audit:inventory
    rtk proxy bun -e "const c=await Bun.file('../tynd/docs/superpowers/audits/2026-07-09-codebase-audit/coverage-ledger.json').json(); const f=await Bun.file('../tynd/docs/superpowers/audits/2026-07-09-codebase-audit/findings.json').json(); const open=c.filter(x=>x.repo==='plugins'&&x.disposition==='pending'); const bad=f.filter(x=>x.repo==='plugins'&&x.status==='confirmed'); if(open.length||bad.length){console.error({open:open.length,bad:bad.length});process.exit(1)}"

Expected: exit 0.

- [ ] **Step 2: Commit plugin audit evidence**

From `tynd`:

    git add docs/superpowers/audits/2026-07-09-codebase-audit
    git commit -m "docs: close official plugin audit coverage"
