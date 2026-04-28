/**
 * E2E smoke test — verifies the full build + spawn round-trip without
 * requiring a display.
 *
 * Strategy: bypass the wry/tao webview entirely with `TYND_SMOKE=1`.
 * Core's `app.start()` checks that env var early and, when set, fires
 * the lifecycle hooks then `process.exit(0)`. That gives us a real
 * exercise of:
 *   1. The TS bundling pipeline (Bun.build over the user's backend
 *      entry, replaying the same flags `@tynd/cli`'s buildFullBundle uses).
 *   2. The bundled backend running under Bun.
 *   3. The lifecycle hooks firing in order (onReady -> onClose).
 *
 * We deliberately do NOT spawn the Rust host binary here. Building it
 * inside `bun test` would slow CI to a crawl (>1 min cold compile);
 * `cargo test` already covers the host side. This smoke test owns the
 * cross-language seam — TypeScript build + JS lifecycle.
 *
 * The bundling logic is duplicated (rather than imported from
 * `@tynd/cli`) on purpose: importing the CLI pulls in `consola`,
 * `pathe`, and friends from a sibling repo whose node_modules may not
 * be hydrated when this test runs from `tynd/`. The duplication is
 * 10 lines and tracked by the upstream test in `cli/`.
 *
 * Run with `bun test e2e/` from the tynd repo root.
 */

import { existsSync, mkdirSync, rmSync } from "node:fs";
import { join, resolve } from "node:path";
import { afterAll, beforeAll, expect, test } from "bun:test";

const FIXTURE_ROOT = resolve(import.meta.dir, "fixtures", "minimal-app");
const ARTIFACT_DIR = resolve(import.meta.dir, ".artifacts");
const BUNDLE_OUT = join(ARTIFACT_DIR, "bundle.dist.js");

beforeAll(() => {
  // Wipe + recreate so a stale build cannot mask a regression.
  if (existsSync(ARTIFACT_DIR)) rmSync(ARTIFACT_DIR, { recursive: true, force: true });
  mkdirSync(ARTIFACT_DIR, { recursive: true });
});

afterAll(() => {
  // Best-effort cleanup. CI runners have ephemeral disks anyway.
  try {
    if (existsSync(ARTIFACT_DIR)) rmSync(ARTIFACT_DIR, { recursive: true, force: true });
  } catch {
    /* ignore — Windows occasionally holds an ESM file lock briefly */
  }
});

test(
  "build pipeline produces a runnable backend bundle",
  async () => {
    const entry = join(FIXTURE_ROOT, "backend", "main.ts");
    expect(existsSync(entry)).toBe(true);

    // Mirror buildFullBundle from @tynd/cli. Kept in sync via PR review;
    // the cli's own bundle tests are the source of truth for flags.
    const result = await Bun.build({
      entrypoints: [entry],
      target: "bun",
      format: "esm",
      minify: true,
      define: { "globalThis.__TYND_RUNTIME__": '"full"' },
    });
    if (!result.success) {
      throw new Error(`bundle failed:\n${result.logs.map((l) => l.message).join("\n")}`);
    }
    const out = result.outputs[0];
    if (!out) throw new Error("Bun.build produced no outputs");
    await Bun.write(BUNDLE_OUT, await out.text());

    expect(existsSync(BUNDLE_OUT)).toBe(true);
    const text = await Bun.file(BUNDLE_OUT).text();
    expect(text.length).toBeGreaterThan(0);
  },
  // Bun.build is ~200 ms typically; pad for cold startup on slow CI.
  60_000,
);

test(
  "smoke run exits cleanly with code 0 under TYND_SMOKE=1",
  async () => {
    expect(existsSync(BUNDLE_OUT)).toBe(true);

    // Spawn the bundled backend under Bun. Core sees TYND_SMOKE=1,
    // fires the lifecycle hooks, and exits 0. No webview, no IPC, no
    // display required — exactly what CI needs.
    const proc = Bun.spawn(["bun", "run", BUNDLE_OUT], {
      env: { ...process.env, TYND_SMOKE: "1", TYND_ENTRY: BUNDLE_OUT },
      stdout: "pipe",
      stderr: "pipe",
    });

    // 30 s is generous — a healthy run completes in <1 s. The timeout
    // mostly catches a regression where TYND_SMOKE stops being honored
    // and the IPC listener wedges on stdin.
    let timer: ReturnType<typeof setTimeout> | undefined;
    const timeout = new Promise<number>((_, reject) => {
      timer = setTimeout(() => {
        try {
          proc.kill();
        } catch {
          /* already dead */
        }
        reject(new Error("smoke binary timed out after 30s"));
      }, 30_000);
    });

    let code: number;
    try {
      code = await Promise.race([proc.exited, timeout]);
    } finally {
      if (timer) clearTimeout(timer);
    }
    expect(code).toBe(0);

    // Sanity: lifecycle hooks should have logged to stderr. If they
    // did not, TYND_SMOKE almost certainly took the wrong branch.
    const stderr = await new Response(proc.stderr).text();
    expect(stderr).toContain("[smoke] onReady fired");
    expect(stderr).toContain("[smoke] onClose fired");
  },
  35_000,
);
