import { fileURLToPath } from "url";
import path from "path";
import os from "os";
import fs from "fs";
import { c } from "./colors";

const CHECK_INTERVAL_MS = 24 * 60 * 60 * 1000; // 24h
const CACHE_FILE = path.join(os.tmpdir(), "bunview-update-check.json");

function compareSemver(a: string, b: string): number {
  const pa = a.split(".").map((n) => parseInt(n, 10));
  const pb = b.split(".").map((n) => parseInt(n, 10));
  for (let i = 0; i < 3; i++) {
    const da = pa[i] ?? 0, db = pb[i] ?? 0;
    if (da > db) return 1;
    if (da < db) return -1;
  }
  return 0;
}

async function readLocalVersion(): Promise<string> {
  const pkgPath = path.join(fileURLToPath(new URL("../../", import.meta.url)), "package.json");
  try {
    const pkg = await Bun.file(pkgPath).json() as { version: string };
    return pkg.version;
  } catch { return "0.0.0"; }
}

/**
 * Non-blocking version check against npm. Prints a notice once per 24h if a
 * newer bunview exists. Silent on network failure. Never blocks the CLI.
 */
export async function maybeNotifyUpdate(): Promise<void> {
  if (process.env.CI || process.env.BUNVIEW_NO_UPDATE_CHECK) return;
  if (!process.stdout.isTTY) return;

  let cached: { checkedAt: number; latest: string } | null = null;
  try {
    cached = JSON.parse(fs.readFileSync(CACHE_FILE, "utf-8"));
  } catch { /* no cache */ }

  const now = Date.now();
  const stale = !cached || (now - cached.checkedAt > CHECK_INTERVAL_MS);

  if (stale) {
    try {
      const res = await fetch("https://registry.npmjs.org/bunview/latest", {
        signal: AbortSignal.timeout(2000),
      });
      if (res.ok) {
        const data = await res.json() as { version: string };
        cached = { checkedAt: now, latest: data.version };
        try { fs.writeFileSync(CACHE_FILE, JSON.stringify(cached)); } catch {}
      }
    } catch { /* offline or slow — skip */ }
  }

  if (!cached) return;
  const current = await readLocalVersion();
  if (compareSemver(cached.latest, current) > 0) {
    console.log(
      `\n${c.yellow("┌─────────────────────────────────────────────┐")}\n` +
      `${c.yellow("│")}  Update available: ${c.dim(current)} → ${c.green(cached.latest)}\n` +
      `${c.yellow("│")}  Run: ${c.cyan("bunview upgrade")}\n` +
      `${c.yellow("└─────────────────────────────────────────────┘")}\n`,
    );
  }
}
