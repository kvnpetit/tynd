# Documentation and Cross-Repository Closure Implementation Plan

> **For Codex:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Align the v1.0 documentation with the implemented runtime, API, plugin, CLI, and configuration contracts; automate the highest-risk drift checks; and close every workspace audit finding with reproducible evidence.

**Architecture:** Keep the documentation repository independently buildable, but place the cross-repository contract checker in `tynd/scripts` because it must compare Tynd source contracts with `../docs` content. Generate no documentation from unstable source formatting. Instead, enforce small semantic invariants and update the prose/examples explicitly.

**Tech Stack:** TypeScript, Bun test runner, Fumadocs, MDX, Knip, Biome, Tynd CLI source contracts

---

## Global Constraints

- Preserve all pre-existing user changes in `tynd`, `plugins`, `mcp`, and `docs`.
- Modify only files named by a task or newly reported by the contract checker.
- Do not document compatibility aliases that the implementation does not accept.
- Use the plugin catalog as the source of truth for plugin package names and installation commands.
- Every confirmed finding must have a regression test or an automated contract check when practical.
- Record each finding and verification command in `tynd/docs/superpowers/audits/2026-07-09-codebase-audit/findings.json`.

### Task 1: Remove the confirmed docs static-analysis debt

**Files:**
- Delete: `D:\Projects\tynd-workspace\docs\src\components\ui\ThemeProvider.tsx`
- Modify: `D:\Projects\tynd-workspace\docs\package.json`
- Modify: `D:\Projects\tynd-workspace\docs\bun.lock`
- Modify: `D:\Projects\tynd-workspace\docs\knip.json`
- Modify: `D:\Projects\tynd-workspace\docs\mdx-components.tsx`

**Step 1: Reproduce the current findings**

Run from `D:\Projects\tynd-workspace\docs`:

```powershell
rtk proxy bun run check:deps
```

Expected: Knip reports the unused ThemeProvider file, direct `tailwind-merge` dependency, MDX exports, and source config entry.

**Step 2: Make the dependency and export boundaries explicit**

- Delete the unused ThemeProvider component after confirming `rg "ThemeProvider" src` has no consumers.
- Remove `tailwind-merge` from direct dependencies; it remains available only through the package that owns its use.
- Add `source.config.ts` to Knip entries.
- Keep `useMDXComponents` local to `mdx-components.tsx` and export only `getMDXComponents` if that is the symbol consumed by the framework.
- Regenerate `bun.lock` with `rtk proxy bun install`.

**Step 3: Verify the cleanup**

```powershell
rtk proxy bun run check:deps
rtk proxy bun run typecheck
rtk proxy bun test
```

Expected: all commands exit 0 and Knip reports no unused file, dependency, or export.

**Step 4: Commit the isolated cleanup**

```powershell
rtk git add src/components/ui/ThemeProvider.tsx package.json bun.lock knip.json mdx-components.tsx
rtk git commit -m "chore: remove unused docs surfaces"
```

### Task 2: Add a semantic docs contract checker

**Files:**
- Create: `D:\Projects\tynd-workspace\tynd\scripts\check-doc-contracts.ts`
- Create: `D:\Projects\tynd-workspace\tynd\scripts\check-doc-contracts.test.ts`
- Modify: `D:\Projects\tynd-workspace\tynd\package.json`

**Step 1: Write failing parser tests**

Create `scripts/check-doc-contracts.test.ts`:

```ts
import { describe, expect, test } from "bun:test";
import {
  extractCoreImports,
  extractSubcommands,
} from "./check-doc-contracts";

describe("documentation contract extraction", () => {
  test("extracts imports from @tynd/core/client", () => {
    expect(
      extractCoreImports(
        'import { invoke, sql } from "@tynd/core/client";',
      ),
    ).toEqual(["invoke", "sql"]);
  });

  test("extracts CLI subcommand keys", () => {
    const source = [
      "subCommands: {",
      "  create: createCommand,",
      '  "config-show": configShowCommand,',
      "},",
    ].join("\n");

    expect(extractSubcommands(source)).toEqual(["create", "config-show"]);
  });
});
```

Run:

```powershell
rtk proxy bun test scripts/check-doc-contracts.test.ts
```

Expected: FAIL because the checker does not exist.

**Step 2: Implement exact semantic checks**

Create `scripts/check-doc-contracts.ts` with these exported parsers and a CLI entry point:

```ts
import { readdir, readFile } from "node:fs/promises";
import { join } from "node:path";

const PLUGIN_FIRST = new Set([
  "audio",
  "fs",
  "httpServer",
  "keyring",
  "llm",
  "oauth",
  "screen",
  "shell",
  "sql",
  "terminal",
  "tts",
  "updater",
  "webview",
  "wgpu",
  "whisper",
  "windowControls",
  "windowState",
  "windowVisual",
  "windows",
]);

const REQUIRED_GLOBAL_FLAGS = [
  "--verbose",
  "--quiet",
  "--no-color",
  "--env",
  "--log-format",
  "--log-file",
];

export function extractCoreImports(source: string): string[] {
  const imports: string[] = [];
  const pattern =
    /import\s*\{([^}]+)\}\s*from\s*["']@tynd\/core\/client["']/g;
  for (const match of source.matchAll(pattern)) {
    for (const item of match[1].split(",")) {
      const name = item.trim().split(/\s+as\s+/)[0];
      if (name) imports.push(name);
    }
  }
  return imports.sort();
}

export function extractSubcommands(source: string): string[] {
  const body = source.match(/subCommands:\s*\{([\s\S]*?)\n\s*\}/)?.[1] ?? "";
  return [...body.matchAll(/^\s*(?:"([^"]+)"|([A-Za-z][\w-]*))\s*:/gm)]
    .map((match) => match[1] ?? match[2])
    .sort();
}

async function walkMdx(root: string): Promise<string[]> {
  const files: string[] = [];
  for (const entry of await readdir(root, { withFileTypes: true })) {
    const path = join(root, entry.name);
    if (entry.isDirectory()) files.push(...(await walkMdx(path)));
    else if (entry.isFile() && path.endsWith(".mdx")) files.push(path);
  }
  return files;
}

async function main(): Promise<void> {
  const workspace = join(import.meta.dir, "..", "..");
  const docsRoot = join(workspace, "docs", "src", "content", "v1.0");
  const cliSource = await readFile(
    join(import.meta.dir, "..", "packages", "cli", "src", "index.ts"),
    "utf8",
  );
  const commands = extractSubcommands(cliSource);
  const files = await walkMdx(docsRoot);
  const failures: string[] = [];
  let allDocs = "";

  for (const file of files) {
    const source = await readFile(file, "utf8");
    allDocs += "\n" + source;
    if (/runtime:\s*["']standard["']|tynd-standard/.test(source)) {
      failures.push(file + ": unsupported standard runtime");
    }
    for (const name of extractCoreImports(source)) {
      if (PLUGIN_FIRST.has(name)) {
        failures.push(
          file + ": " + name + " must not be imported from @tynd/core/client",
        );
      }
    }
  }

  for (const command of commands) {
    if (!allDocs.includes("tynd " + command)) {
      failures.push("undocumented CLI command: " + command);
    }
  }
  for (const flag of REQUIRED_GLOBAL_FLAGS) {
    if (!allDocs.includes(flag)) {
      failures.push("undocumented global flag: " + flag);
    }
  }

  if (failures.length > 0) {
    throw new Error(failures.sort().join("\n"));
  }
}

if (import.meta.main) await main();
```

**Step 3: Run the tests and capture the expected documentation failures**

```powershell
rtk proxy bun test scripts/check-doc-contracts.test.ts
rtk proxy bun scripts/check-doc-contracts.ts
```

Expected: parser tests pass; the CLI check fails on the known runtime, import, command, and flag drift.

**Step 4: Add the stable package script**

Add to `tynd/package.json`:

```json
"docs:contracts": "bun scripts/check-doc-contracts.ts"
```

Run `rtk proxy bun run docs:contracts` and confirm it still fails until Tasks 3–5 are complete.

**Step 5: Commit the checker**

```powershell
rtk git add scripts/check-doc-contracts.ts scripts/check-doc-contracts.test.ts package.json
rtk git commit -m "test: enforce documentation contracts"
```

### Task 3: Correct the runtime and API architecture documentation

**Files:**
- Modify: `D:\Projects\tynd-workspace\docs\src\content\v1.0\concepts\runtimes.mdx`
- Modify: `D:\Projects\tynd-workspace\docs\src\content\v1.0\api\index.mdx`
- Modify: `D:\Projects\tynd-workspace\docs\src\content\v1.0\api\plugins.mdx`
- Modify: `D:\Projects\tynd-workspace\docs\src\content\v1.0\concepts\architecture.mdx`

**Step 1: Replace the nonexistent runtime**

- Document only `full`, `lite`, `full-custom`, and `lite-custom`.
- Explain that custom variants change the embedded runtime assembly while retaining the corresponding full/lite contract.
- Remove every `standard` and `tynd-standard` example.
- Copy accepted values and defaults from `tynd/packages/cli/src/lib/config/schema.ts`.

**Step 2: Replace API-parity claims with the implemented boundary**

- State that core provides the stable bootstrap, IPC, event, and plugin-loading primitives.
- State that 19 specialized capabilities are plugin-first.
- List all plugin-first symbols enforced by `PLUGIN_FIRST` in Task 2.
- Link each capability family to the exact package and install command from `tynd/docs/plugins/catalog.json`.
- Remove claims that lite and full expose every specialized API without plugins.

**Step 3: Verify architecture prose**

Run from `D:\Projects\tynd-workspace\docs`:

```powershell
rg -n 'standard|tynd-standard|API parity|same APIs' src/content/v1.0
```

Expected: no unsupported runtime or false parity statement remains.

**Step 4: Commit the architecture correction**

Run from `D:\Projects\tynd-workspace\docs`:

```powershell
rtk git add src/content/v1.0/concepts/runtimes.mdx src/content/v1.0/api/index.mdx src/content/v1.0/api/plugins.mdx src/content/v1.0/concepts/architecture.mdx
rtk git commit -m "docs: align runtime and API architecture"
```

### Task 4: Correct plugin-first imports and installation examples

**Files:**
- Modify: `D:\Projects\tynd-workspace\docs\src\content\v1.0\api\os\fs.mdx`
- Modify: `D:\Projects\tynd-workspace\docs\src\content\v1.0\api\os\keyring.mdx`
- Modify: `D:\Projects\tynd-workspace\docs\src\content\v1.0\api\os\shell.mdx`
- Modify: `D:\Projects\tynd-workspace\docs\src\content\v1.0\api\os\updater.mdx`
- Modify: `D:\Projects\tynd-workspace\docs\src\content\v1.0\api\os\web.mdx`
- Modify: `D:\Projects\tynd-workspace\docs\src\content\v1.0\api\os\windowState.mdx`
- Modify: `D:\Projects\tynd-workspace\docs\src\content\v1.0\concepts\architecture.mdx`
- Modify: `D:\Projects\tynd-workspace\docs\src\content\v1.0\concepts\security.mdx`
- Modify: `D:\Projects\tynd-workspace\docs\src\content\v1.0\guides\auto-updates.mdx`
- Modify: `D:\Projects\tynd-workspace\docs\src\content\v1.0\guides\backend-rpc.mdx`
- Modify: `D:\Projects\tynd-workspace\docs\src\content\v1.0\guides\binary-data.mdx`
- Modify: `D:\Projects\tynd-workspace\docs\src\content\v1.0\guides\migrate-from-electron.mdx`
- Modify: `D:\Projects\tynd-workspace\docs\src\content\v1.0\guides\migrate-from-tauri.mdx`
- Modify: `D:\Projects\tynd-workspace\docs\src\content\v1.0\guides\persistence.mdx`
- Modify: `D:\Projects\tynd-workspace\docs\src\content\v1.0\recipes\remember-window-size.mdx`
- Modify: `D:\Projects\tynd-workspace\docs\src\content\v1.0\tutorials\chat-client.mdx`
- Modify: `D:\Projects\tynd-workspace\docs\src\content\v1.0\tutorials\file-browser.mdx`
- Modify: `D:\Projects\tynd-workspace\docs\src\content\v1.0\tutorials\markdown-editor.mdx`
- Reference: `D:\Projects\tynd-workspace\tynd\docs\plugins\catalog.json`

**Step 1: Produce the exact failing-file list**

Run from `D:\Projects\tynd-workspace\tynd`:

```powershell
rtk proxy bun run docs:contracts
```

Record every `@tynd/core/client` plugin-first import in the audit ledger before editing.

**Step 2: Use package-owned entry points**

For each reported import:

- Keep `invoke`, `listen`, and other core primitives on `@tynd/core/client`.
- Import specialized APIs from their package-owned client entry point.
- Add the catalog’s exact package installation command immediately before the first example.
- Keep examples executable: package name, imported symbol, and capability name must agree.

**Step 3: Prove no stale imports remain**

```powershell
rtk proxy bun run docs:contracts
```

Expected: plugin-first import failures are gone; remaining failures are only undocumented CLI commands or flags until Task 5.

**Step 4: Commit the example corrections**

Run from `D:\Projects\tynd-workspace\docs`:

```powershell
rtk git add src/content/v1.0
rtk git commit -m "docs: use plugin-owned client APIs"
```

### Task 5: Complete the CLI, flags, and configuration reference

**Files:**
- Modify: `D:\Projects\tynd-workspace\docs\src\content\v1.0\cli\commands.mdx`
- Modify: `D:\Projects\tynd-workspace\docs\src\content\v1.0\cli\config.mdx`
- Modify: `D:\Projects\tynd-workspace\docs\src\content\v1.0\cli\index.mdx`
- Reference: `D:\Projects\tynd-workspace\tynd\packages\cli\src\index.ts`
- Reference: `D:\Projects\tynd-workspace\tynd\packages\cli\src\lib\config\schema.ts`

**Step 1: Document the exact command registry**

Add concise syntax and purpose for all implemented commands:

`create`, `add`, `dev`, `start`, `build`, `init`, `clean`, `validate`, `upgrade`, `config-show`, `info`, `explain`, `size`, `fix`, `doctor`, `keygen`, `sign`, `updater-keygen`, `publish`, `release`, `report`, `plugin-new`, `plugin-add`, `plugin-remove`, `plugin-check`, `plugin-dev`, `plugin-list`, `gen-types`, `profile`, `update-server`, and `completions`.

Use each command module’s `help`, `examples`, and argument schema as the authoritative wording.

**Step 2: Document all global flags**

Document `--verbose` with its `--debug` alias, `--quiet`, `--no-color`, `--env`, `--log-format`, and `--log-file`, including precedence and accepted values from `packages/cli/src/index.ts`.

**Step 3: Reconcile configuration fields**

- Compare every public field in `schema.ts` with `cli/config.mdx`.
- Add missing defaults and accepted enum values.
- Remove unsupported fields and obsolete aliases.
- Keep runtime values identical to Task 3.

**Step 4: Make the contract checker green**

Run from `D:\Projects\tynd-workspace\tynd`:

```powershell
rtk proxy bun run docs:contracts
rtk proxy bun test scripts/check-doc-contracts.test.ts
```

Expected: both commands exit 0.

**Step 5: Commit the CLI reference**

Run from `D:\Projects\tynd-workspace\docs`:

```powershell
rtk git add src/content/v1.0/cli/commands.mdx src/content/v1.0/cli/config.mdx src/content/v1.0/cli/index.mdx
rtk git commit -m "docs: complete CLI and config reference"
```

### Task 6: Run final documentation and cross-repository closure gates

**Files:**
- Modify: `D:\Projects\tynd-workspace\tynd\docs\superpowers\audits\2026-07-09-codebase-audit\findings.json`
- Modify: `D:\Projects\tynd-workspace\tynd\docs\superpowers\audits\2026-07-09-codebase-audit\coverage-ledger.json`
- Create: `D:\Projects\tynd-workspace\tynd\docs\superpowers\audits\2026-07-09-codebase-audit\final-report.md`

**Step 1: Run the full docs gates**

Run from `D:\Projects\tynd-workspace\docs`:

```powershell
rtk proxy bun run verify:full
```

Expected: typecheck, lint, dependency analysis, tests, links, and production build all pass.

**Step 2: Run cross-repository contract checks**

Run from `D:\Projects\tynd-workspace\tynd`:

```powershell
rtk proxy bun run docs:contracts
rtk proxy bun run audit:inventory
```

Expected: documentation contracts pass and the audit ledger reports no open confirmed finding.

**Step 3: Re-run each repository’s full verification**

Run `rtk proxy bun run verify:full` separately from:

- `D:\Projects\tynd-workspace\tynd`
- `D:\Projects\tynd-workspace\plugins`
- `D:\Projects\tynd-workspace\mcp`
- `D:\Projects\tynd-workspace\docs`

Expected: every repository exits 0. If a repository has no `verify:full` script, run the exact typecheck, lint, dependency, test, Rust, and build commands recorded in `coverage-ledger.json`.

**Step 4: Write the final report**

In `final-report.md` include:

- scope and exclusions;
- baseline failures;
- confirmed findings and root causes;
- fixes and refactors;
- regression tests added;
- commands and exit codes;
- unresolved observations explicitly classified as non-findings;
- remaining risks, if any.

Mark a finding `closed` only when its regression command and owning package gate both pass.

**Step 5: Verify ledger completeness**

```powershell
rtk proxy bun run audit:inventory
rg -n '"status":\s*"open"|"status":\s*"in_progress"' docs/superpowers/audits/2026-07-09-codebase-audit
```

Expected: audit command exits 0 and ripgrep returns no open or in-progress confirmed finding.

**Step 6: Commit closure evidence**

```powershell
rtk git add docs/superpowers/audits/2026-07-09-codebase-audit/findings.json docs/superpowers/audits/2026-07-09-codebase-audit/coverage-ledger.json docs/superpowers/audits/2026-07-09-codebase-audit/final-report.md
rtk git commit -m "docs: close workspace quality audit"
```
