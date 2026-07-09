# Gate Stabilization and Audit Ledger Implementation Plan

> **For Codex:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Restore trustworthy workspace gates and create the central evidence ledger used to prove complete file and finding coverage.

**Architecture:** Keep the four Git repositories independent while placing cross-repository audit artifacts and tooling in the primary `tynd` repository. Fix the three reproduced gate defects with narrow test seams before starting subsystem remediation.

**Tech Stack:** Bun 1.3.x, TypeScript 6, Bun test, Git, JSON audit artifacts, existing repository scripts.

## Global Constraints

- Current staged, unstaged, and untracked user changes are authoritative and must remain intact.
- No public TypeScript or Rust API break.
- No global write formatter, reset, checkout, or clean command.
- Generated and mirrored files are changed only through their authoritative generator.
- Production code follows red-green-refactor; formatting/config-only changes use focused before/after gates.
- Every command runs from the repository stated by the task.
- Commit only paths owned by the current task.

---

### Task 1: Add the cross-repository coverage ledger generator

**Files:**
- Create: `scripts/audit-workspace.ts`
- Create: `scripts/audit-workspace.test.ts`
- Create on first run: `docs/superpowers/audits/2026-07-09-codebase-audit/coverage-ledger.json`
- Create on first run: `docs/superpowers/audits/2026-07-09-codebase-audit/findings.json`
- Modify: `package.json`

**Interfaces:**
- Consumes: four sibling repositories at the workspace root and `git ls-files --cached --others --exclude-standard`.
- Produces: deterministic `CoverageRow[]` and `Finding[]` JSON contracts plus `bun run audit:inventory`.

- [ ] **Step 1: Write failing classification and merge tests**

Create `scripts/audit-workspace.test.ts`:

```ts
import { describe, expect, test } from "bun:test";
import { classifyPath, mergeCoverageRows, subsystemFor } from "./audit-workspace";

describe("workspace audit inventory", () => {
  test("classifies source, tests, docs, config, generated, and mirrors", () => {
    expect(classifyPath("packages/core/src/index.ts")).toBe("source");
    expect(classifyPath("packages/core/src/index.test.ts")).toBe("test");
    expect(classifyPath("src/content/v1.0/index.mdx")).toBe("documentation");
    expect(classifyPath(".github/workflows/ci.yml")).toBe("config");
    expect(classifyPath("audio/audio.gen.ts")).toBe("generated");
    expect(classifyPath("packages/host-src/native/src/lib.rs")).toBe("mirror");
  });

  test("derives stable subsystem names", () => {
    expect(subsystemFor("tynd", "packages/core/src/index.ts")).toBe("packages/core");
    expect(subsystemFor("plugins", "audio/native/src/lib.rs")).toBe("audio/native");
    expect(subsystemFor("mcp", "src/bridge/client.ts")).toBe("src/bridge");
    expect(subsystemFor("docs", "src/content/v1.0/index.mdx")).toBe("src/content");
  });

  test("preserves reviewed dispositions while adding new files", () => {
    const previous = [{
      repo: "tynd",
      path: "packages/core/src/index.ts",
      fileClass: "source" as const,
      subsystem: "packages/core",
      reviewMethod: ["bun run typecheck"],
      disposition: "clean" as const,
      findingIds: [],
      evidence: ["typecheck pass"],
    }];
    const next = mergeCoverageRows(previous, [
      { repo: "tynd", path: "packages/core/src/index.ts" },
      { repo: "tynd", path: "packages/core/src/new.ts" },
    ]);
    expect(next[0]?.disposition).toBe("clean");
    expect(next[1]?.disposition).toBe("pending");
  });
});
```

- [ ] **Step 2: Run the focused test and verify RED**

Run from `D:\Projects\tynd-workspace\tynd`:

    rtk proxy bun test scripts/audit-workspace.test.ts

Expected: FAIL because `./audit-workspace` and its exports do not exist.

- [ ] **Step 3: Implement the deterministic inventory**

Create `scripts/audit-workspace.ts`:

```ts
import { existsSync, mkdirSync } from "node:fs";
import path from "node:path";
import { spawnSync } from "node:child_process";

export type FileClass =
  | "source"
  | "test"
  | "config"
  | "script"
  | "documentation"
  | "generated"
  | "mirror";

export type CoverageDisposition =
  | "pending"
  | "clean"
  | "finding"
  | "generated-verified"
  | "duplicate-justified"
  | "deferred";

export interface CoverageRow {
  repo: string;
  path: string;
  fileClass: FileClass;
  subsystem: string;
  reviewMethod: string[];
  disposition: CoverageDisposition;
  findingIds: string[];
  evidence: string[];
}

export interface Finding {
  id: string;
  repo: string;
  title: string;
  category: string;
  severity: "critical" | "high" | "medium" | "low";
  confidence: "high" | "medium" | "low";
  locations: Array<{ path: string; line: number }>;
  reproduction: string[];
  rootCause: string;
  invariant: string;
  status: "candidate" | "confirmed" | "false-positive" | "fixed" | "deferred";
  validation: string[];
}

interface InventoryPath {
  repo: string;
  path: string;
}

const REPOS = ["tynd", "plugins", "mcp", "docs"] as const;
const GENERATED = /(?:^|\/)(?:dist|coverage|target|generated)(?:\/|$)|\.gen\.ts$|\.snap$|\/llms(?:-full)?\.txt$/;
const TEST = /(?:^|\/)(?:test|tests)(?:\/|$)|\.(?:test|spec)\.[cm]?[jt]sx?$/;
const CONFIG = /(?:^|\/)(?:\.github|\.cargo)(?:\/|$)|(?:^|\/)(?:package|tsconfig|biome|knip)\.jsonc?$|\.(?:toml|ya?ml|jsonc?)$/;

export function classifyPath(relativePath: string): FileClass {
  const normalized = relativePath.replaceAll("\\", "/");
  if (normalized.startsWith("packages/host-src/native/")) return "mirror";
  if (GENERATED.test(normalized)) return "generated";
  if (TEST.test(normalized)) return "test";
  if (/\.(?:md|mdx|txt)$/.test(normalized)) return "documentation";
  if (normalized.startsWith("scripts/")) return "script";
  if (CONFIG.test(normalized)) return "config";
  return "source";
}

export function subsystemFor(repo: string, relativePath: string): string {
  const parts = relativePath.replaceAll("\\", "/").split("/");
  if (repo === "tynd" && parts[0] === "packages") return parts.slice(0, 2).join("/");
  if (repo === "plugins") return parts.slice(0, 2).join("/");
  if (repo === "mcp" && parts[0] === "src") return parts.slice(0, 2).join("/");
  if (repo === "docs" && parts[0] === "src") return parts.slice(0, 2).join("/");
  return parts[0] ?? ".";
}

function pendingRow(item: InventoryPath): CoverageRow {
  return {
    ...item,
    fileClass: classifyPath(item.path),
    subsystem: subsystemFor(item.repo, item.path),
    reviewMethod: [],
    disposition: "pending",
    findingIds: [],
    evidence: [],
  };
}

export function mergeCoverageRows(
  previous: CoverageRow[],
  inventory: InventoryPath[],
): CoverageRow[] {
  const saved = new Map(previous.map((row) => [`${row.repo}:${row.path}`, row]));
  return inventory
    .map((item) => saved.get(`${item.repo}:${item.path}`) ?? pendingRow(item))
    .sort((a, b) => a.repo.localeCompare(b.repo) || a.path.localeCompare(b.path));
}

function listRepoFiles(repoRoot: string, repo: string): InventoryPath[] {
  const result = spawnSync(
    "git",
    ["ls-files", "--cached", "--others", "--exclude-standard", "-z"],
    { cwd: repoRoot, encoding: "utf8", windowsHide: true },
  );
  if (result.status !== 0) {
    throw new Error(`git ls-files failed for ${repo}: ${result.stderr.trim()}`);
  }
  return result.stdout
    .split("\0")
    .filter(Boolean)
    .map((entry) => ({ repo, path: entry.replaceAll("\\", "/") }));
}

async function readJson<T>(file: string, fallback: T): Promise<T> {
  if (!existsSync(file)) return fallback;
  return (await Bun.file(file).json()) as T;
}

export async function main(): Promise<void> {
  const workspaceRoot = path.resolve(import.meta.dir, "../..");
  const auditDir = path.resolve(
    import.meta.dir,
    "../docs/superpowers/audits/2026-07-09-codebase-audit",
  );
  mkdirSync(auditDir, { recursive: true });

  const inventory = REPOS.flatMap((repo) =>
    listRepoFiles(path.join(workspaceRoot, repo), repo),
  );
  const coveragePath = path.join(auditDir, "coverage-ledger.json");
  const findingsPath = path.join(auditDir, "findings.json");
  const previous = await readJson<CoverageRow[]>(coveragePath, []);
  const findings = await readJson<Finding[]>(findingsPath, []);

  await Bun.write(coveragePath, `${JSON.stringify(mergeCoverageRows(previous, inventory), null, 2)}\n`);
  await Bun.write(findingsPath, `${JSON.stringify(findings, null, 2)}\n`);
  console.log(`inventory: ${inventory.length} files; findings: ${findings.length}`);
}

if (import.meta.main) await main();
```

Add to root `package.json` scripts:

```json
"audit:inventory": "bun run scripts/audit-workspace.ts",
"audit:test": "bun test scripts/audit-workspace.test.ts"
```

- [ ] **Step 4: Verify GREEN and deterministic output**

    rtk proxy bun test scripts/audit-workspace.test.ts
    rtk proxy bun run audit:inventory
    rtk proxy bun run audit:inventory
    rtk git diff -- docs/superpowers/audits/2026-07-09-codebase-audit

Expected: tests PASS; second inventory run produces no additional diff.

- [ ] **Step 5: Commit only ledger tooling and initial artifacts**

    git add scripts/audit-workspace.ts scripts/audit-workspace.test.ts package.json docs/superpowers/audits/2026-07-09-codebase-audit
    git commit -m "test: add workspace audit coverage ledger"

---

### Task 2: Make the Tynd bundle regression test cross-drive portable

**Files:**
- Modify: `packages/cli/src/lib/bundle.test.ts`

**Interfaces:**
- Consumes: `buildLiteBundle(entryPath, outPath, minify, dev, opts)`.
- Produces: a portable file-URL module specifier for the existing test fixture.

- [ ] **Step 1: Reproduce RED on the normal Windows temp drive**

    rtk proxy bun test packages/cli/src/lib/bundle.test.ts --test-name-pattern "lite runtime define keeps Bun-only startup code out of core bundles"

Expected: FAIL with `Could not resolve: "./D:/Projects/.../packages/core/src/index.ts"` when temp and repository use different drives.

- [ ] **Step 2: Replace relative-path construction with a file URL**

Add:

```ts
import { pathToFileURL } from "node:url";
```

Replace the `coreIndex` construction and generated import with:

```ts
const coreIndex = pathToFileURL(path.join(root, "packages/core/src/index.ts")).href;
const entry = path.join(dir, "entry.ts");
writeFileSync(
  entry,
  [
    `import { app } from ${JSON.stringify(coreIndex)};`,
    "app.start({ window: { title: 'Lite' } });",
    "export const ping = () => 'pong';",
  ].join("\n"),
);
```

- [ ] **Step 3: Verify GREEN on default and same-drive temp locations**

    rtk proxy bun test packages/cli/src/lib/bundle.test.ts --test-name-pattern "lite runtime define keeps Bun-only startup code out of core bundles"
    $env:TEMP='D:\Projects\tynd-workspace\tynd'; $env:TMP=$env:TEMP; rtk proxy bun test packages/cli/src/lib/bundle.test.ts --test-name-pattern "lite runtime define keeps Bun-only startup code out of core bundles"
    rtk proxy bun test packages/cli/src/lib/bundle.test.ts

Expected: all commands PASS; generated bundle contains none of `import.meta.require`, `process.stdin`, `TYND_IPC_FORMAT`, or `CborFrameReader`.

- [ ] **Step 4: Commit the portable fixture**

    git add packages/cli/src/lib/bundle.test.ts
    git commit -m "test(cli): make lite bundle fixture cross-drive safe"

---

### Task 3: Replace the recursive MCP module mock with dependency injection

**Files:**
- Modify: `..\mcp\src\tools\app.ts`
- Modify: `..\mcp\test\regressions.test.ts`

**Interfaces:**
- Consumes: Node `spawn` with its existing signature.
- Produces: `registerAppTools(context, spawnProcess?)`; one optional compatible test seam.

- [ ] **Step 1: Reproduce the hanging test**

Run from `D:\Projects\tynd-workspace\mcp`:

    rtk proxy bun test test/regressions.test.ts --test-name-pattern "two parallel launches result in only one spawn"

Expected: command does not terminate within the bounded test window because the mock restoration factory imports its own active module mock.

- [ ] **Step 2: Add an optional spawn dependency**

In `src/tools/app.ts`, add:

```ts
export type SpawnProcess = typeof spawn;
```

Change the registration signature:

```ts
export function registerAppTools(
  { server, runtime }: ToolContext,
  spawnProcess: SpawnProcess = spawn,
): void {
```

Replace only the launch call:

```ts
child = spawnProcess(bin, ["dev"], {
  cwd: path,
  detached: true,
  stdio: "ignore",
  shell: false,
  env: filterSensitiveEnv(process.env),
});
```

- [ ] **Step 3: Rewrite the regression test without `mock.module`**

Import:

```ts
import { registerAppTools, type SpawnProcess } from "../src/tools/app.ts";
```

Replace the module mock and `finally` restoration with:

```ts
const spawnProcess = (() => {
  spawnCount += 1;
  return makeChild();
}) as SpawnProcess;

process.env.TYND_BIN = process.execPath;
try {
  const runtime = createRuntime({ allowLaunch: true });
  const tools = new Map<string, (args: unknown) => Promise<unknown>>();
  const fakeServer = {
    registerTool(name: string, _meta: unknown, fn: (args: unknown) => Promise<unknown>) {
      tools.set(name, fn);
    },
  };
  registerAppTools(
    {
      // biome-ignore lint/suspicious/noExplicitAny: test stub exposes registerTool only
      server: fakeServer as any,
      runtime,
    },
    spawnProcess,
  );
  const launch = tools.get("app.launch");
  if (!launch) throw new Error("app.launch not registered");
  const [a, b] = await Promise.all([
    launch({ path: process.cwd() }),
    launch({ path: process.cwd() }),
  ]);
  expect(spawnCount).toBe(1);
  const errors = [a, b].filter(
    (value) => (value as { isError?: boolean }).isError === true,
  );
  expect(errors).toHaveLength(1);
} finally {
  delete process.env.TYND_BIN;
}
```

Use the imported `registerAppTools` in the two earlier app-launch tests and remove their dynamic imports.

- [ ] **Step 4: Verify termination and behavior**

    rtk proxy bun test test/regressions.test.ts --test-name-pattern "app.launch cooldown gate"
    rtk proxy bun test test/regressions.test.ts
    rtk proxy bun test
    rtk proxy bun run typecheck

Expected: exactly one spawn; competing call returns `already in flight`; every command terminates and passes.

- [ ] **Step 5: Commit the seam and regression**

    git add src/tools/app.ts test/regressions.test.ts
    git commit -m "test(mcp): inject app launch process spawner"

---

### Task 4: Restore reproducible plugin workspace peer metadata

**Files:**
- Create: `..\plugins\scripts\check-peer-metadata.test.ts`
- Modify: all thirteen `..\plugins\<package>\package.json` manifests.
- Verify: `..\plugins\bun.lock`.

**Interfaces:**
- Consumes: every workspace package's `peerDependencies`.
- Produces: matching `peerDependenciesMeta.<name>.optional = true` entries so development install does not fetch unpublished peers.

- [ ] **Step 1: Add a failing workspace metadata test**

Create `scripts/check-peer-metadata.test.ts`:

```ts
import { describe, expect, test } from "bun:test";
import path from "node:path";

const root = path.resolve(import.meta.dir, "..");
const workspace = (await Bun.file(path.join(root, "package.json")).json()) as {
  workspaces: string[];
};

describe("plugin peer metadata", () => {
  for (const dir of workspace.workspaces) {
    test(`${dir} marks every peer optional for workspace development`, async () => {
      const pkg = (await Bun.file(path.join(root, dir, "package.json")).json()) as {
        peerDependencies?: Record<string, string>;
        peerDependenciesMeta?: Record<string, { optional?: boolean }>;
      };
      for (const peer of Object.keys(pkg.peerDependencies ?? {})) {
        expect(pkg.peerDependenciesMeta?.[peer]?.optional).toBe(true);
      }
    });
  }
});
```

- [ ] **Step 2: Verify RED**

Run from `D:\Projects\tynd-workspace\plugins`:

    rtk proxy bun test scripts/check-peer-metadata.test.ts

Expected: FAIL because current manifests omit `peerDependenciesMeta`.

- [ ] **Step 3: Restore metadata represented by the lockfile**

For every package, add immediately after `peerDependencies`:

```json
"peerDependenciesMeta": {
  "@tynd/core": { "optional": true },
  "@tynd/plugin-sdk": { "optional": true }
}
```

Use only the peers actually present in that package:

- `command-palette` and `gpu`: `@tynd/core`.
- `audio`, `http-server`, `keyring`, `oauth`, `screen-rec`, `sql`, `terminal`, `tts`, `whisper`: `@tynd/core` and `@tynd/plugin-sdk`.
- `llm`: the two Tynd peers plus `@ai-sdk/provider` and `ai`.
- `wgpu`: the two Tynd peers plus `three`.

Every listed key uses `{ "optional": true }`.

- [ ] **Step 4: Verify GREEN and frozen installation**

    rtk proxy bun test scripts/check-peer-metadata.test.ts
    rtk proxy bun install --frozen-lockfile
    rtk proxy bun run verify:quick
    rtk git diff -- bun.lock

Expected: tests and frozen install PASS; lockfile either remains unchanged or changes only to match manifest peer metadata. No registry request for unpublished Tynd peers.

- [ ] **Step 5: Commit manifests and lock only**

    git add scripts/check-peer-metadata.test.ts */package.json bun.lock
    git commit -m "fix(plugins): restore optional peer metadata"

---

### Task 5: Record baseline findings and close Wave 0

**Files:**
- Modify: `docs/superpowers/audits/2026-07-09-codebase-audit/coverage-ledger.json`
- Modify: `docs/superpowers/audits/2026-07-09-codebase-audit/findings.json`

**Interfaces:**
- Consumes: verified results from Tasks 2–4.
- Produces: stable finding identifiers `TYND-001`, `MCP-001`, and `PLUGINS-001` with fixed status and exact commands.

- [ ] **Step 1: Add the three finding records**

Use titles:

- `TYND-001 — Cross-drive lite bundle fixture creates invalid import specifier`.
- `MCP-001 — Recursive Bun module mock prevents regression suite termination`.
- `PLUGINS-001 — Missing optional peer metadata breaks frozen install`.

Each record must include the red command, root cause above, changed paths, green commands, and `status: "fixed"` only after all green evidence exists.

- [ ] **Step 2: Attach coverage rows**

Mark the exact changed source/test/config rows `finding` and attach their finding IDs. Keep all unrelated rows `pending`.

- [ ] **Step 3: Run Wave 0 gates**

    rtk proxy bun run audit:inventory
    rtk proxy bun run verify:quick
    rtk proxy bun --cwd ..\mcp run verify:quick
    rtk proxy bun --cwd ..\plugins run verify:quick

Expected: every command terminates and passes.

- [ ] **Step 4: Commit Wave 0 evidence**

    git add docs/superpowers/audits/2026-07-09-codebase-audit
    git commit -m "docs: record gate stabilization evidence"
