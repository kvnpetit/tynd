# MCP Audit and Remediation Implementation Plan

> **For Codex:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Prove the MCP server terminates deterministically, preserves bridge and tool contracts, and closes every source/config/test coverage row.

**Architecture:** Treat discovery, transport/handshake, JSON-RPC, runtime attachment, and MCP surfaces as separate ownership domains. Validate each domain with isolated tests before the full suite, then verify catalog synchronization against sibling repositories.

**Tech Stack:** Bun 1.3.x, TypeScript 6, MCP SDK, Node sockets/processes, tsdown, Biome, Knip.

## Global Constraints

- Execute the Wave 0 MCP spawn-injection task first.
- Preserve MCP tool/resource/prompt names, JSON-RPC wire shapes, size limits, and default safety gates.
- Every socket, server, timer, child process, subscription, and module mock must have deterministic cleanup.
- `webview.evaluate` remains opt-in; launch and non-loopback gates remain default-deny.
- No source change without a focused failing test and root-cause evidence.

---

### Task 1: Prove every MCP test file terminates independently and together

**Files:**
- Verify all fourteen current test files.
- Modify tests only when a specific handle leak or shared-state dependency is reproduced.

**Interfaces:**
- Consumes: Bun test lifecycle.
- Produces: isolated and aggregate termination receipts.

- [ ] **Step 1: Run every test file independently**

    rtk proxy bun test src/bridge/transport-handshake.test.ts
    rtk proxy bun test test/bridge-client-coverage.test.ts
    rtk proxy bun test test/bridge-protocol.test.ts
    rtk proxy bun test test/cli-call.test.ts
    rtk proxy bun test test/cli-coverage.test.ts
    rtk proxy bun test test/discovery.test.ts
    rtk proxy bun test test/e2e/mock-host.test.ts
    rtk proxy bun test test/handshake.test.ts
    rtk proxy bun test test/regressions.test.ts
    rtk proxy bun test test/resources-prompts-coverage.test.ts
    rtk proxy bun test test/server-coverage.test.ts
    rtk proxy bun test test/tools-live-coverage.test.ts
    rtk proxy bun test test/tools-prompts-extra.test.ts
    rtk proxy bun test test/tools-static.test.ts

Expected: every command exits 0 without external termination.

- [ ] **Step 2: Run aggregate suite repeatedly**

    rtk proxy bun test
    rtk proxy bun test
    rtk proxy bun test

Expected: three clean exits with identical pass/skip counts.

- [ ] **Step 3: Record lifecycle evidence**

For each test file, record owned sockets/servers/timers/subscriptions and the hook or explicit close that releases them. Mark the fixed `regressions.test.ts` row with `MCP-001`.

---

### Task 2: Audit discovery and liveness boundaries

**Files:**
- Inspect: `..\mcp\src\discovery\filter.ts`
- Inspect: `..\mcp\src\discovery\paths.ts`
- Inspect: `..\mcp\src\discovery\pid-check.ts`
- Inspect: `..\mcp\src\discovery\scan.ts`
- Inspect: `..\mcp\test\discovery.test.ts`

**Interfaces:**
- Consumes: discovery JSON files, process IDs, timestamps, loopback addresses, cookies.
- Produces: validated live-entry selection and stale-entry handling.

- [ ] **Step 1: Run discovery tests with warnings visible**

    rtk proxy bun test test/discovery.test.ts

- [ ] **Step 2: Review full discovery data flow**

Trace:

```text
debug directory -> file enumeration -> JSON parse -> shape validation
-> host/cookie/timestamp validation -> PID liveness -> newest entry selection
-> runtime attach
```

Prove exact bounds for file count, file size, cookie shape, timestamp age, loopback host, and stale PID handling. Confirm malformed files are skipped without aborting the scan.

- [ ] **Step 3: Record findings before patching**

Any missing bound or incorrect platform assumption gets a candidate with exact fixture and expected behavior. Add a complete red-green task to this plan before changing source.

---

### Task 3: Audit transport, handshake, framing, and JSON-RPC

**Files:**
- Inspect: `..\mcp\src\bridge\client.ts`
- Inspect: `..\mcp\src\bridge\framing.ts`
- Inspect: `..\mcp\src\bridge\protocol.ts`
- Inspect: `..\mcp\src\bridge\transport-handshake.ts`
- Inspect: `..\mcp\src\bridge\transport-socket.ts`
- Inspect: `..\mcp\src\bridge\transport-tcp.ts`
- Inspect: `..\mcp\src\bridge\transport-uds.ts`
- Inspect: associated bridge/handshake tests.

**Interfaces:**
- Consumes: length-prefixed frames and authenticated hello messages.
- Produces: bounded request/response/subscription behavior with one close notification.

- [ ] **Step 1: Run focused transport tests**

    rtk proxy bun test src/bridge/transport-handshake.test.ts test/handshake.test.ts test/bridge-protocol.test.ts test/bridge-client-coverage.test.ts

- [ ] **Step 2: Verify invariants**

Prove:

- frame cap is checked before allocation;
- zero/partial/concatenated frames behave deterministically;
- handshake listeners are registered before send;
- frames arriving during client swap are replayed once;
- cookie comparison is constant-time for equal-length bytes;
- close/error fires once and rejects all pending requests;
- request IDs cannot collide or change type;
- timeouts/cancellation release pending-map entries;
- response byte caps count UTF-8 bytes.

- [ ] **Step 3: Record full-file receipts**

Attach exact tests or static evidence to every bridge source row. Create candidates for uncovered branches before implementation.

---

### Task 4: Audit runtime attachment and server lifecycle

**Files:**
- Inspect: `..\mcp\src\runtime.ts`
- Inspect: `..\mcp\src\server.ts`
- Inspect: `..\mcp\src\util\limits.ts`
- Inspect: `..\mcp\src\util\logger.ts`
- Inspect: `..\mcp\test\server-coverage.test.ts`

**Interfaces:**
- Consumes: discovery scans and bridge clients.
- Produces: one active attachment, deterministic reattach/detach, and bounded MCP outputs.

- [ ] **Step 1: Run server coverage tests**

    rtk proxy bun test test/server-coverage.test.ts test/tools-live-coverage.test.ts

- [ ] **Step 2: Review state transitions**

Document transitions:

```text
unattached -> attaching -> attached -> connection lost
-> rescan -> reattached or unattached -> shutdown
```

Verify old bridges, scan timers, subscriptions, and server transports are closed on every transition.

- [ ] **Step 3: Verify limits**

Trace every user-controlled `n`, `maxBytes`, payload, screenshot, log, and IPC response through schema validation and serialization cap. Record exact constant and test for each surface.

---

### Task 5: Audit MCP tools, resources, prompts, and CLI

**Files:**
- Inspect all files under `..\mcp\src\tools`, `resources`, `prompts`, `cli`, and `catalog`.
- Inspect registration and coverage tests.

**Interfaces:**
- Consumes: MCP schemas and optional live bridge.
- Produces: stable registered names, safe unattached fallbacks, and actionable errors.

- [ ] **Step 1: Run surface tests**

    rtk proxy bun test test/tools-static.test.ts test/tools-prompts-extra.test.ts test/tools-live-coverage.test.ts test/resources-prompts-coverage.test.ts test/cli-call.test.ts test/cli-coverage.test.ts

- [ ] **Step 2: Compare registration to exported catalog**

    rtk grep "registerTool(" src/tools --glob "*.ts"
    rtk grep "registerResource(" src/resources --glob "*.ts"
    rtk grep "registerPrompt(" src/prompts --glob "*.ts"

Build a list of registered names and confirm every name has schema, description, unattached behavior, bridge method, size cap, and test.

- [ ] **Step 3: Review dangerous operations**

Prove launch, eval, replay, quit/restart, filesystem paths, and non-loopback discovery remain gated. Confirm error results never silently fall through to static catalog success.

- [ ] **Step 4: Review CLI parsing**

Confirm `serve`, `call`, and `tap` reject partial numeric values, malformed JSON, unknown flags, and missing values; ensure one-shot commands close bridges.

---

### Task 6: Verify catalog synchronization, build, and coverage

**Files:**
- Verify: `..\mcp\scripts\preflight-catalog.ts`
- Verify: `..\mcp\scripts\sync-catalog.ts`
- Verify generated catalog files under `..\mcp\src\catalog`.

- [ ] **Step 1: Run repository gates**

From `D:\Projects\tynd-workspace\mcp`:

    rtk proxy bun install --frozen-lockfile
    rtk proxy bun run lint
    rtk proxy bun run deps
    rtk proxy bun run typecheck
    rtk proxy bun run test
    rtk proxy bun run coverage:ci
    rtk proxy bun run build

- [ ] **Step 2: Verify sibling catalog sources**

    rtk proxy bun run preflight-catalog
    rtk proxy bun run sync-catalog
    rtk git diff -- src/catalog

Expected: preflight passes and synchronization produces no unexpected drift. Any intended generated change is committed with its authoritative sibling change.

- [ ] **Step 3: Close MCP ledger**

From `tynd`:

    rtk proxy bun run audit:inventory
    rtk proxy bun -e "const c=await Bun.file('docs/superpowers/audits/2026-07-09-codebase-audit/coverage-ledger.json').json(); const f=await Bun.file('docs/superpowers/audits/2026-07-09-codebase-audit/findings.json').json(); const open=c.filter(x=>x.repo==='mcp'&&x.disposition==='pending'); const bad=f.filter(x=>x.repo==='mcp'&&x.status==='confirmed'); if(open.length||bad.length){console.error({open:open.length,bad:bad.length});process.exit(1)}"

Expected: exit 0.

- [ ] **Step 4: Commit MCP audit receipts**

    git add docs/superpowers/audits/2026-07-09-codebase-audit
    git commit -m "docs: close MCP audit coverage"
