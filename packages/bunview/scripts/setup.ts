#!/usr/bin/env bun
/**
 * Postinstall — downloads the pre-compiled webview-host from GitHub Releases.
 *
 * Env vars:
 *   BUNVIEW_BUILD_FROM_SOURCE=1   skip download, compile from source instead
 *   BUNVIEW_SKIP_BUILD=1          skip entirely (CI runners building the host itself)
 */

import path from "path";
import { chmodSync } from "fs";
import { fileURLToPath } from "url";

// ── Target detection ──────────────────────────────────────────────────────────

type Target =
  | "windows-x64" | "windows-arm64"
  | "linux-x64"   | "linux-arm64"
  | "macos-x64"   | "macos-arm64";

function currentTarget(): Target {
  const arch = process.arch === "arm64" ? "arm64" : "x64";
  if (process.platform === "win32")  return `windows-${arch}`;
  if (process.platform === "darwin") return `macos-${arch}`;
  return `linux-${arch}`;
}

const TARGET = currentTarget();
const EXT    = TARGET.startsWith("windows") ? ".exe" : "";

const ROOT    = path.join(import.meta.dir, "..");
const BIN_DIR = path.join(ROOT, "bin", TARGET);
const BIN_OUT = path.join(BIN_DIR, `webview-host${EXT}`);

// ── Helpers ───────────────────────────────────────────────────────────────────

/** Extract "owner/repo" from a git repository URL. */
function parseOwnerRepo(url: string): string | null {
  const m = url.match(/github\.com[/:]([\w.-]+\/[\w.-]+?)(?:\.git)?$/);
  return m ? m[1] : null;
}

async function buildFromSource(): Promise<void> {
  const script = fileURLToPath(new URL("./build-host.ts", import.meta.url));
  const proc = Bun.spawn(["bun", script], {
    stdout: "inherit",
    stderr: "inherit",
    env: { ...process.env, BUNVIEW_TARGET: TARGET },
  });
  const code = await proc.exited;
  if (code !== 0) process.exit(code);
}

// ── Main ──────────────────────────────────────────────────────────────────────

// 1. Skip entirely (e.g. CI building the host itself)
if (process.env.BUNVIEW_SKIP_BUILD === "1") {
  process.exit(0);
}

// 2. Binary already present — nothing to do
if (await Bun.file(BIN_OUT).exists()) {
  process.exit(0);
}

// 3. Explicit "build from source" flag
if (process.env.BUNVIEW_BUILD_FROM_SOURCE === "1") {
  await buildFromSource();
  process.exit(0);
}

// 4. Download from GitHub Releases
const pkgPath    = path.join(ROOT, "package.json");
const pkg        = await Bun.file(pkgPath).json() as { repository?: { url?: string } };
const repoUrl    = pkg.repository?.url ?? "";
const ownerRepo  = parseOwnerRepo(repoUrl);

if (!ownerRepo) {
  console.warn(
    "[bunview] ⚠  Cannot determine GitHub repository from package.json.\n" +
    "          Set repository.url to your GitHub repo URL, or set BUNVIEW_BUILD_FROM_SOURCE=1.",
  );
  console.warn("[bunview] Falling back to building from source…");
  await buildFromSource();
  process.exit(0);
}

const assetName  = `webview-host-${TARGET}${EXT}`;
const downloadUrl = `https://github.com/${ownerRepo}/releases/latest/download/${assetName}`;

let downloaded = false;
try {
  const res = await fetch(downloadUrl);
  if (!res.ok) {
    throw new Error(`HTTP ${res.status} ${res.statusText}`);
  }

  await Bun.write(BIN_OUT, await res.arrayBuffer());

  // Make executable on Linux/macOS
  if (process.platform !== "win32") {
    chmodSync(BIN_OUT, 0o755);
  }

  downloaded = true;
} catch (err: unknown) {
  const msg = err instanceof Error ? err.message : String(err);
  console.warn(
    `[bunview] ⚠  Failed to download webview-host from GitHub Releases: ${msg}\n` +
    `          URL: ${downloadUrl}\n` +
    `          Falling back to building from source — a C++ toolchain is required.`,
  );
}

if (!downloaded) {
  await buildFromSource();
}
