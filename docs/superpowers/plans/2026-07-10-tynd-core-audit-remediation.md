# Tynd Core Audit and Remediation Implementation Plan

> **For Codex:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Close the Tynd repository's static-quality failures, validate every core subsystem, and record evidence for every in-scope file without changing public behavior.

**Architecture:** Treat embedded JavaScript, TypeScript packages, and Rust crates according to their real runtime boundaries. Fix configuration drift first, then semantic lints, then format each affected Rust package in bounded commits before running complete gates.

**Tech Stack:** Bun 1.3.x, TypeScript 6, Biome 2.4.14, Rust 1.94, nightly rustfmt, Clippy, Cargo tests and coverage.

## Global Constraints

- Execute `2026-07-10-gate-stabilization-audit-ledger.md` first.
- Current worktree changes remain authoritative; inspect each diff before applying a patch.
- Preserve public exports, IPC formats, package formats, lite/full runtime behavior, and host-source synchronization.
- Embedded JavaScript is not mechanically modernized merely to satisfy browser-style lint rules.
- No broad all-repository formatter; format one owned package/task at a time.
- Each semantic change receives a failing regression test or existing reproducible lint/test proof.
- Update the central coverage and finding ledgers after every task.

---

### Task 1: Make Biome scope follow authoritative embedded-JS locations

**Files:**
- Create: `scripts/check-biome-scope.test.ts`
- Modify: `biome.json`

**Interfaces:**
- Consumes: Biome `files.includes` exclusions.
- Produces: exact exclusions for raw `include_str!` assets and a regression test rejecting stale exact exclusions.

- [ ] **Step 1: Write the failing scope test**

```ts
import { describe, expect, test } from "bun:test";
import { existsSync } from "node:fs";
import path from "node:path";

const root = path.resolve(import.meta.dir, "..");
const config = (await Bun.file(path.join(root, "biome.json")).json()) as {
  files: { includes: string[] };
};
const includes = config.files.includes;

describe("Biome embedded asset scope", () => {
  test("excludes current raw embedded JavaScript assets", () => {
    expect(includes).toContain("!packages/ipc-wire/src/devalue.bundle.js");
    expect(includes).toContain("!packages/ipc-wire/src/error_overlay.js");
    expect(includes).toContain("!packages/quickjs-core/src/globals.js");
  });

  test("every non-glob exact exclusion still exists", () => {
    for (const rule of includes.filter((value) => value.startsWith("!"))) {
      const relative = rule.slice(1);
      if (/[*?[\]{}]/.test(relative)) continue;
      expect(existsSync(path.join(root, relative)), relative).toBe(true);
    }
  });
});
```

- [ ] **Step 2: Verify RED**

    rtk proxy bun test scripts/check-biome-scope.test.ts

Expected: FAIL for the three new locations and obsolete exact exclusions.

- [ ] **Step 3: Update `biome.json`**

Remove obsolete exact entries:

```json
"!packages/host-rs/src/error_overlay.js",
"!packages/host-rs/src/devalue.bundle.js"
```

Add:

```json
"!packages/ipc-wire/src/devalue.bundle.js",
"!packages/ipc-wire/src/error_overlay.js",
"!packages/quickjs-core/src/globals.js"
```

Keep `packages/host-rs/src/debug/debug_helper.js` because that file still exists.

- [ ] **Step 4: Verify GREEN and the diagnostic reduction**

    rtk proxy bun test scripts/check-biome-scope.test.ts
    rtk proxy bunx biome check --max-diagnostics=200

Expected: the test passes; Biome no longer reports 119 errors and 48 warnings from the three embedded assets. Remaining diagnostics are the focused TypeScript/script items in Task 2.

- [ ] **Step 5: Commit scope only**

    git add biome.json scripts/check-biome-scope.test.ts
    git commit -m "chore: align Biome scope with embedded assets"

---

### Task 2: Close the remaining TypeScript and script diagnostics

**Files:**
- Modify: `packages/cli/src/commands/dev.ts`
- Modify: `packages/cli/src/commands/profile.ts`
- Modify: `packages/cli/src/index.ts`
- Modify: `packages/cli/src/lib/bundle.ts`
- Modify: `packages/cli/src/lib/config.ts`
- Modify: `packages/cli/src/lib/config/schema.test.ts`
- Modify: `packages/cli/src/lib/runtime-manifests.test.ts`
- Modify: `packages/core/src/config.test.ts`
- Modify: `packages/core/src/config.ts`
- Modify: `packages/core/src/index.test.ts`
- Modify: `scripts/cargo-with-windows-test-env.ts`

**Interfaces:**
- Consumes: current behavior and existing test suite.
- Produces: zero Biome errors/warnings without changing test cleanup semantics or CLI stderr behavior.

- [ ] **Step 1: Capture RED**

    rtk proxy bunx biome check packages/cli packages/core scripts --max-diagnostics=200

Expected: 12 errors and one warning across the files listed above.

- [ ] **Step 2: Apply only safe organization and formatting fixes**

    rtk proxy bunx biome check --write packages/cli/src/commands/dev.ts packages/cli/src/commands/profile.ts packages/cli/src/index.ts packages/cli/src/lib/bundle.ts packages/cli/src/lib/config.ts packages/cli/src/lib/config/schema.test.ts packages/cli/src/lib/runtime-manifests.test.ts packages/core/src/config.test.ts packages/core/src/config.ts

Expected: imports and formatting change; no unsafe `noDelete` or `noConsole` removal.

- [ ] **Step 3: Document intentional test cleanup and stderr behavior**

Use:

```ts
onUpdate: () => {
  // Required callback; this test observes the thrown configuration error only.
},
```

Before both global cleanup deletes:

```ts
// biome-ignore lint/performance/noDelete: test must restore property absence, not assign undefined
delete (globalThis as Record<string, unknown>).__TYND_RUNTIME__;
// biome-ignore lint/performance/noDelete: test must restore property absence, not assign undefined
delete (globalThis as Record<string, unknown>).__tynd_config__;
```

Before the usage error:

```ts
// biome-ignore lint/suspicious/noConsole: wrapper usage errors belong on stderr
console.error("usage: bun run scripts/cargo-with-windows-test-env.ts <cargo-args...>");
```

- [ ] **Step 4: Verify tests and Biome**

    rtk proxy bun test packages/core/src/config.test.ts packages/core/src/index.test.ts packages/cli/src/lib/config/schema.test.ts packages/cli/src/lib/runtime-manifests.test.ts
    rtk proxy bun run typecheck
    rtk proxy bunx biome check packages/cli packages/core scripts --max-diagnostics=200

Expected: all tests/typecheck pass; zero diagnostics in these scopes.

- [ ] **Step 5: Commit the bounded TypeScript cleanup**

    git add packages/cli/src packages/core/src scripts/cargo-with-windows-test-env.ts
    git commit -m "chore: close TypeScript quality diagnostics"

---

### Task 3: Fix or justify all sixteen Clippy diagnostics

**Files:**
- Modify: `packages/pack-format/src/lib.rs`
- Modify: `packages/update-core/src/lib.rs`
- Modify: `packages/debug-protocol/src/ipc_tap.rs`
- Modify: `packages/debug-protocol/src/log_tap.rs`
- Modify: `packages/debug-protocol/src/server.rs`
- Modify: `packages/ipc-wire/src/lib.rs`
- Modify: `packages/capabilities/src/lib.rs`
- Test: existing tests in the same crates.

**Interfaces:**
- Consumes: unchanged serialization/config/public APIs.
- Produces: zero Clippy warnings with explicit rationale only where a public serialized shape makes a lint inapplicable.

- [ ] **Step 1: Capture RED**

    rtk cargo clippy --workspace --all-targets -- -D warnings

Expected: 16 diagnostics across the seven files above.

- [ ] **Step 2: Make pack parsing conversions explicit**

Use:

```rust
let trailer_len = i64::try_from(TRAILER_LEN).ok()?;
f.seek(SeekFrom::End(-trailer_len)).ok()?;
```

Replace `entry.rel.as_bytes().len()` with `entry.rel.len()`. Run:

    rtk cargo test -p tynd-pack-format

- [ ] **Step 3: Preserve updater wire shape and make extension matching explicit**

Add immediately above `UpdaterConfig`:

```rust
#[allow(clippy::struct_excessive_bools)] // Public camelCase config mirrors the TypeScript schema.
```

Replace allocation-based suffix detection with:

```rust
let extension = url.rsplit_once('.').map(|(_, extension)| extension);
if extension.is_some_and(|extension| {
    extension.eq_ignore_ascii_case("zst") || extension.eq_ignore_ascii_case("zstd")
}) {
    Ok(Self::Zstd)
} else {
    Ok(Self::None)
}
```

Add assertions for `artifact.ZST` and `artifact.ZsTd` to the existing compression test, first verifying they fail before the implementation change.

Run:

    rtk cargo test -p tynd-update-core

- [ ] **Step 4: Remove duplicate match arms in debug taps**

In both tap files use:

```rust
senders.retain(|sender| {
    !matches!(
        sender.try_send(frame.clone()),
        Err(crossbeam_channel::TrySendError::Disconnected(_))
    )
});
```

In `server.rs` replace the two match-return patterns with:

```rust
let Ok(body) = encode(&note) else {
    return false;
};
let Ok(mut writer) = writer_for_emit.lock() else {
    return false;
};
write_frame(&mut **writer, &body).is_ok()
```

Run:

    rtk cargo test -p tynd-debug-protocol

- [ ] **Step 5: Use explicit clones and modern Option helpers**

In the IPC property test:

```rust
.map(|(id, value)| (id.clone(), value.clone()))
```

In capabilities:

```rust
platforms.is_none_or(|allowed| allowed.contains(&target))
window.is_none_or(|value| cmd.windows.iter().any(|pattern| pattern.matches(value)))
webview.is_none_or(|value| cmd.webviews.iter().any(|pattern| pattern.matches(value)))
Value::default()
```

Retain `split_permission_ref`'s tuple-producing `map_or` unless Clippy still reports it; it is not equivalent to `is_none_or`.

Run:

    rtk cargo test -p tynd-ipc-wire
    rtk cargo test -p tynd-capabilities

- [ ] **Step 6: Verify GREEN**

    rtk cargo clippy --workspace --all-targets -- -D warnings
    rtk cargo test -p tynd-pack-format -p tynd-update-core -p tynd-debug-protocol -p tynd-ipc-wire -p tynd-capabilities

Expected: zero warnings and all focused tests pass.

- [ ] **Step 7: Commit semantic lint fixes**

    git add packages/pack-format packages/update-core packages/debug-protocol packages/ipc-wire packages/capabilities
    git commit -m "refactor(rust): close strict Clippy diagnostics"

---

### Task 4: Close rustfmt drift by owned package, not one global rewrite

**Files:**
- Modify only rustfmt-reported files under `packages/capabilities`, `packages/host-rs`, `packages/plugin-sdk/native`, `packages/quickjs-core`, and `packages/security-policy`.

**Interfaces:**
- Consumes: rustfmt 63-hunk baseline.
- Produces: unchanged behavior and a green workspace format check.

- [ ] **Step 1: Record the current 63-hunk RED output**

    rtk proxy bun run format:check:rust

Expected: exit 1, 63 hunks across 44 files.

- [ ] **Step 2: Format one package at a time and inspect each diff**

    cargo +nightly fmt -p tynd-capabilities
    rtk git diff -- packages/capabilities
    cargo +nightly fmt -p tynd-host
    rtk git diff -- packages/host-rs
    cargo +nightly fmt -p tynd_plugin_sdk
    rtk git diff -- packages/plugin-sdk/native
    cargo +nightly fmt -p tynd-quickjs-core
    rtk git diff -- packages/quickjs-core
    cargo +nightly fmt -p tynd-security-policy
    rtk git diff -- packages/security-policy

Expected: whitespace/import ordering only. Stop if rustfmt changes generated or unrelated files.

- [ ] **Step 3: Verify behavior after formatting**

    rtk cargo test -p tynd-capabilities -p tynd-host -p tynd_plugin_sdk -p tynd-quickjs-core -p tynd-security-policy
    rtk proxy bun run format:check:rust

Expected: tests pass and format check exits 0.

- [ ] **Step 4: Commit formatting separately**

    git add packages/capabilities packages/host-rs packages/plugin-sdk/native packages/quickjs-core packages/security-policy
    git commit -m "style(rust): apply workspace formatting"

---

### Task 5: Complete the Tynd file-by-file audit receipts

**Files:**
- Modify: `docs/superpowers/audits/2026-07-09-codebase-audit/coverage-ledger.json`
- Modify: `docs/superpowers/audits/2026-07-09-codebase-audit/findings.json`

**Interfaces:**
- Consumes: all Tynd inventory rows and automated evidence.
- Produces: no `pending` Tynd rows and bounded follow-up implementation plans for every newly confirmed source finding before it is patched.

- [ ] **Step 1: Run automated repository evidence**

    rtk proxy bun run audit:inventory
    rtk proxy bun run check:all
    rtk proxy bun run deps:all
    rtk proxy bun run verify:full
    rtk proxy bun run coverage:ci
    rtk proxy bun run coverage:rust:ci
    rtk proxy bun run sync:host-src:check

- [ ] **Step 2: Review runtime/control boundaries in dependency order**

Review full files, not snippets:

1. leaf crates: `capabilities`, `protocol`, `ipc-wire`, `pack-format`, `update-core`, `security-policy`;
2. `host-core` and `host-macros`;
3. `host-rs` app, IPC, scheme, runtime, plugins, and each `os` module;
4. `quickjs-core`, `lite`, and `full`;
5. `core` TypeScript API/runtime;
6. `cli` build/dev/publish/update/plugin pipelines;
7. `plugin-sdk` and `host-src` synchronization.

For each file, add exact review methods and evidence. Mark generated/mirror rows only after their generator or sync check passes.

- [ ] **Step 3: Stop before patching newly discovered findings**

For each new candidate, record source, sink/control, reproduction, invariant, severity, and proof gaps in `findings.json`. Confirm root cause and add an exact red-green task to this plan or a dedicated addendum before editing production code.

- [ ] **Step 4: Prove Tynd ledger closure**

Run a Bun one-liner that exits nonzero if any `repo === "tynd"` row remains `pending` or any confirmed finding is not `fixed`:

    rtk proxy bun -e "const c=await Bun.file('docs/superpowers/audits/2026-07-09-codebase-audit/coverage-ledger.json').json(); const f=await Bun.file('docs/superpowers/audits/2026-07-09-codebase-audit/findings.json').json(); const open=c.filter(x=>x.repo==='tynd'&&x.disposition==='pending'); const bad=f.filter(x=>x.repo==='tynd'&&x.status==='confirmed'); if(open.length||bad.length){console.error({open:open.length,bad:bad.length});process.exit(1)}"

Expected: exit 0.

- [ ] **Step 5: Commit Tynd audit receipts**

    git add docs/superpowers/audits/2026-07-09-codebase-audit
    git commit -m "docs: close Tynd audit coverage"

---

### Task 6: Run final Tynd gates

**Files:**
- Verify only.

- [ ] **Step 1: Run complete gates**

    rtk proxy bun install --frozen-lockfile
    rtk proxy bun run check:all
    rtk proxy bun run deps:all
    rtk proxy bun run verify:full
    rtk proxy bun run coverage:ci
    rtk proxy bun run coverage:rust:ci
    rtk proxy bun run sync:host-src:check
    rtk proxy bun --cwd packages/cli run build:all
    rtk proxy bun --cwd packages/plugin-sdk run verify:full

- [ ] **Step 2: Record exact command evidence**

Append exit codes, pass counts, coverage percentages, and any platform proof gaps to the finding and coverage ledgers. Do not claim closure from a partial gate.
