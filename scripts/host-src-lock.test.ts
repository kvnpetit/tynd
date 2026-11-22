import { afterAll, expect, test } from "bun:test";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const ROOT = join(import.meta.dir, "..");
const out = mkdtempSync(join(tmpdir(), "tynd-host-src-lock-"));

afterAll(() => rmSync(out, { recursive: true, force: true }));

interface LockPackage {
  name: string;
  version: string;
  source?: string;
  checksum?: string;
}

function registryPins(lockPath: string): Set<string> {
  const doc = Bun.TOML.parse(readFileSync(lockPath, "utf8")) as { package?: LockPackage[] };
  return new Set(
    (doc.package ?? [])
      .filter((p) => p.source !== undefined)
      .map((p) => `${p.name} ${p.version} ${p.source} ${p.checksum ?? ""}`),
  );
}

// Regression: the sync used to copy a hand-maintained lock next to freshly
// generated manifests, so any dependency change left a lock that Cargo
// rejected under --locked.
test("sync:host-src writes a lock Cargo accepts with --locked, pinned to the root lock", () => {
  const sync = Bun.spawnSync(
    ["bun", "run", join(ROOT, "packages", "host-src", "scripts", "sync.ts"), "--output", out],
    { stdout: "ignore", stderr: "pipe" },
  );
  expect(sync.stderr.toString()).not.toContain("error");
  expect(sync.exitCode).toBe(0);

  // Fails if the lock is not already up to date for the generated manifests.
  const locked = Bun.spawnSync(
    ["cargo", "update", "--workspace", "--locked", "--manifest-path", join(out, "Cargo.toml")],
    { stdout: "ignore", stderr: "pipe" },
  );
  expect(locked.stderr.toString()).not.toContain("--locked was passed");
  expect(locked.exitCode).toBe(0);

  const rootPins = registryPins(join(ROOT, "Cargo.lock"));
  const mirrorPins = [...registryPins(join(out, "Cargo.lock"))];
  expect(mirrorPins.length).toBeGreaterThan(0);
  expect(mirrorPins.filter((p) => !rootPins.has(p))).toEqual([]);
}, 180_000);
