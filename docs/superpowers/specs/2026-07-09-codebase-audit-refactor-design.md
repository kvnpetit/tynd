# Tynd Workspace Codebase Audit and Refactor Design

Date: 2026-07-09
Status: design approved; written specification pending user review
Primary workspace: D:\Projects\tynd-workspace

## Purpose

Audit the complete current workspace, confirm bugs and maintainability defects
with evidence, fix every confirmed finding, and improve code structure without
breaking public behavior. Completion requires fresh proof from the repositories'
tests, linters, type checkers, dependency checks, coverage gates, builds, and
cross-repository consistency checks.

The current working trees are the authoritative baseline. Existing staged,
unstaged, and untracked user changes must be preserved.

## Scope

The workspace contains four independent Git repositories:

| Repository | Primary responsibilities | Main technologies |
| --- | --- | --- |
| tynd | framework core, CLI, TypeScript API, Rust hosts, runtimes, plugin SDK | Bun, TypeScript, Rust |
| plugins | official JavaScript and native plugins | Bun, TypeScript, Rust |
| mcp | MCP server, discovery, bridge, tools, resources, prompts | Bun, TypeScript |
| docs | versioned documentation site and generated reference content | React, TanStack Start, Fumadocs, Cloudflare |

Every tracked or current untracked source, test, configuration, script, and
documentation file is in scope. Generated or mirrored files remain in scope for
drift verification, but their authoritative source must be changed instead of
editing generated output by hand.

Known generated boundaries include:

- tynd/packages/host-src/native, synchronized from the host source.
- MCP catalog snapshots assembled from the sibling repositories.
- docs/public llms files generated from versioned documentation.
- generated plugin bindings and snapshots.

## Non-goals

- No public API break or intentional product behavior change.
- No broad formatting-only rewrite.
- No dependency upgrade unless required to fix a confirmed finding.
- No speculative abstraction without a demonstrated maintenance problem.
- No manual edit to generated or mirrored artifacts.
- No cleanup, reset, or replacement of pre-existing user changes.
- No security-specific scan workflow; ordinary correctness and quality review
  may still identify security-relevant defects.

## Current Baseline Evidence

Initial read-only checks established:

| Repository | Evidence | Initial result |
| --- | --- | --- |
| docs | bun run verify:quick | Pass: Biome, typecheck, and 9 tests |
| tynd | bun run verify:quick; bun run check; Knip; mirror sync | Typecheck passed; 785 tests passed and 1 failed; Biome reported 131 errors and 49 warnings; Knip and mirror sync passed |
| plugins | bun run verify:quick | Pass after partial dependency installation; 6 tests passed |
| mcp | tsc and isolated Bun tests | Typecheck passed; most isolated tests passed, one test process hangs |

The initial confirmed or strongly evidenced gate findings are:

1. Tynd Windows test-fixture path defect. A bundle test creates its temporary
   entry on C: while importing repository source from D:. path.relative returns
   a cross-drive absolute path, after which the fixture prepends ./ and produces
   the invalid specifier ./D:/.... The same test passes when its temporary
   directory is placed on D:. Production bundling is not implicated by current
   evidence.
2. MCP test isolation defect. The parallel app.launch regression test replaces
   node:child_process with mock.module and tries to restore it with a factory
   that imports node:child_process again. That import resolves through the
   active module mock and never completes. The single test and the full suite
   hang; adjacent isolated test groups pass.
3. Plugins clean-install defect. bun install --frozen-lockfile requests
   @tynd/plugin-sdk from the public registry and receives HTTP 404. A partial
   install leaves enough tooling for the quick gate to pass, but a fresh clone
   is not reproducibly installable from the current workspace state.
4. Tynd static-quality backlog. Biome currently reports 131 errors and 49
   warnings, including unsorted imports, formatting drift, an empty block,
   discouraged delete usage, QuickJS JavaScript diagnostics, and a noConsole
   warning. These diagnostics require individual classification: mechanical
   style findings may be fixed in bounded path-scoped batches, while semantic
   findings require tests and root-cause analysis.

These findings are inputs to the implementation plan, not substitutes for the
complete repository audit.

## Audit Architecture

### Coverage ledger

The implementation will maintain a central machine-readable or tabular coverage
ledger under
tynd/docs/superpowers/audits/2026-07-09-codebase-audit/ from the workspace
root. Each in-scope file receives one row with:

- repository and relative path;
- file class: source, test, config, script, documentation, generated, or mirror;
- owning subsystem;
- review method and commands;
- disposition: clean, finding, generated-verified, duplicate-justified, or
  deferred;
- finding identifiers where applicable;
- evidence or exact reason for any deferral.

The goal cannot be completed while any file row lacks a disposition. Automated
checks may cover repetitive content or generated output, while runtime and
high-risk source files require direct review receipts.

### Finding ledger

Every candidate receives a stable identifier and records:

- repository, subsystem, files, and exact lines;
- category: correctness, reliability, portability, testability, performance,
  maintainability, dependency hygiene, documentation drift, or build/release;
- observed symptom and reproduction command;
- root cause and counterevidence;
- public invariant that must remain true;
- severity and confidence;
- status: candidate, confirmed, false positive, fixed, or deferred;
- regression or characterization test;
- focused and repository-level verification commands.

A candidate is fixed only when the original symptom no longer reproduces and
the relevant repository gate passes. False positives require explicit
counterevidence.

### Review layers

Each repository is reviewed through complementary layers:

1. Reproducible setup and baseline gates.
2. Static checks: Biome, TypeScript, Clippy, rustfmt, Knip, Cargo Shear, and
   repository-specific validators.
3. Test and coverage review, including hangs, skips, flaky timing, weak mocks,
   and uncovered boundary behavior.
4. Direct source review by subsystem, prioritizing entry points, parsers,
   concurrency, filesystem/process/network boundaries, caches, generated-code
   pipelines, and high-churn or oversized modules.
5. Cross-file and cross-repository contract review.
6. Build, packaging, publication, and generated-artifact drift review.
7. Documentation-to-code consistency review.

No broad sink count, file size metric, or passing test suite alone proves a
subsystem clean.

## Execution Waves

### Wave 0: Restore trustworthy gates

Fix the three confirmed baseline defects first:

- portable Tynd bundle test fixture;
- isolated MCP spawn dependency or equivalent non-recursive test seam;
- reproducible local dependency resolution for the plugins workspace.

Each fix follows red-green-refactor. Existing failures provide the red state;
additional tests are added where the regression is not fully encoded.

### Wave 1: Tynd core platform

Audit the TypeScript and Rust workspaces by bounded subsystem:

- CLI and build/publish pipeline;
- core TypeScript API and runtime wire formats;
- host contracts, macros, and host integration;
- full and lite runtimes;
- capability, protocol, IPC, update, pack, and policy crates;
- plugin SDK and generated host-source synchronization;
- system plugin facades.

Large modules are split only after characterization coverage exists and only
along concrete responsibility boundaries. The host-source mirror is refreshed
through its synchronization command.

### Wave 2: Official plugins

Audit each plugin as an independent package, then audit shared conventions:

- JavaScript-only command-palette and GPU packages;
- each native plugin's TypeScript facade, generated binding, Rust dispatch,
  state/resource lifecycle, and platform-specific behavior;
- workspace dependency and release contracts;
- repeated native-lib patterns suitable for SDK-level extraction.

Monolithic native lib.rs files are not split mechanically. Extraction requires
a named responsibility, stable interface, and characterization tests.

### Wave 3: MCP

Audit:

- discovery and process-liveness logic;
- UDS/TCP transports, framing, handshake, and JSON-RPC client;
- server lifecycle and reattachment;
- tool and resource registration;
- output limits and error translation;
- CLI commands and catalog synchronization.

Long-running tests must terminate deterministically. Timers, sockets, mock
modules, and spawned children require explicit ownership and cleanup.

### Wave 4: Documentation

Audit:

- route and version metadata;
- code/API names against current manifests and exported surfaces;
- internal links and anchors;
- Cloudflare/Vite build behavior;
- schema and generated llms output;
- duplicated versioned content and intentional divergence.

Generated text is regenerated only through documented scripts, then checked for
drift.

### Wave 5: Cross-repository closure

Verify local development and release contracts in dependency order:

1. tynd and plugin SDK;
2. plugins using the local/published contract;
3. MCP catalog aggregation;
4. docs schemas, imports, and current terminology.

Fresh frozen installs and repository gates must work without relying on partial
node_modules state.

## Remediation Rules

### Correctness fixes

For each confirmed bug:

1. Reproduce consistently and capture exact failure.
2. Trace the cause to the earliest incorrect boundary.
3. Add or retain the smallest failing regression test.
4. Implement one minimal fix.
5. Prove legitimate behavior still works.
6. Search sibling call sites for the same defect.
7. Run focused and repository-level gates.

### Refactors

Refactors require behavior characterization first. A refactor is justified by
at least one concrete condition:

- a module has multiple independently changing responsibilities;
- duplicated logic has already diverged or creates repeated defect risk;
- dependency direction violates documented architecture;
- hidden global state, timers, mocks, sockets, or process handles prevent
  deterministic testing;
- an interface exposes implementation details or makes invariants difficult to
  enforce;
- generated and authoritative sources are confused.

Line count and churn are prioritization signals, not sufficient justification.
Refactors preserve public exports, error contracts, serialization formats, and
runtime behavior unless a confirmed bug requires correction.

### Dirty-worktree safety

- Record status before and after each bounded task.
- Never reset, checkout, clean, or discard unrelated changes.
- Never run global write-formatters.
- Use path-scoped patches and tests.
- Commit only explicitly owned paths when a commit is required.
- Stop and reconcile if a task overlaps an unknown concurrent user edit.

## Error Handling and Compatibility

Boundary failures must be explicit and actionable. Fixes should reject invalid
input at the narrowest authoritative boundary and preserve valid inputs.
Timeouts and retries are used only when the failure is genuinely external or
timing-dependent; they must not mask deterministic bugs.

Compatibility invariants include:

- existing public TypeScript and Rust APIs remain source-compatible;
- lite and full runtime contracts remain stable;
- IPC and package formats remain backward-compatible;
- plugin identifiers and generated bindings stay synchronized;
- Windows, macOS, and Linux behavior remains represented by tests or explicit
  platform evidence;
- documentation names match current code and manifests.

## Verification Matrix

Focused tests run after every change. At wave closure, run the strongest
available gates.

### tynd

    bun install --frozen-lockfile
    bun run check:all
    bun run deps:all
    bun run verify:full
    bun run coverage:ci
    bun run coverage:rust:ci
    bun run sync:host-src:check
    bun run --cwd packages/cli build:all
    bun run --cwd packages/plugin-sdk verify:full

### plugins

    bun install --frozen-lockfile
    bun run check:all
    bun run deps:all
    bun run verify:full
    bun run coverage:ci
    bun run coverage:rust

### mcp

    bun install --frozen-lockfile
    bun run verify:full
    bun run coverage:ci
    bun run preflight-catalog

### docs

    bun install --frozen-lockfile
    bun run verify:full
    bun run coverage:ci

Platform-specific or unavailable commands require exact failure evidence and a
documented proof gap; they do not silently become successful.

## Completion Criteria

The goal is complete only when all conditions hold:

1. Every in-scope file has a closed coverage-ledger row.
2. Every candidate has a proven false-positive disposition, a verified fix, or
   an explicit unavoidable deferral accepted by the user.
3. Every confirmed bug has a regression test or strongest repeatable validation
   artifact.
4. Every targeted refactor has characterization coverage and preserves public
   behavior.
5. No known sibling instance of a fixed defect remains open.
6. Frozen installation succeeds from a clean dependency state in all four
   repositories.
7. Relevant lint, typecheck, dependency, test, coverage, build, synchronization,
   link, and generated-drift gates pass with fresh evidence.
8. Cross-repository contracts and current documentation agree.
9. Existing unrelated user changes remain intact.
10. Final report lists exact files changed, findings closed, commands run,
    results, and any remaining proof gaps.

## Planning Decomposition

After user review of this specification, implementation planning is split into
five independently reviewable plans stored under
tynd/docs/superpowers/plans/ from the workspace root:

1. Gate stabilization and audit-ledger scaffolding.
2. Tynd core audit and remediation.
3. Plugins audit and remediation.
4. MCP audit and remediation.
5. Documentation and cross-repository closure.

Each plan uses small test-driven tasks with exact files, commands, expected
failures, interfaces, and verification gates. The full workspace goal remains
active until all five plans and the completion criteria are closed.
