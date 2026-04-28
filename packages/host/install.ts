#!/usr/bin/env bun
/**
 * Postinstall script for @tynd/host.
 *
 * Downloads the `tynd-full` + `tynd-lite` binaries for the host platform
 * from the GitHub Release matching this package's version and places
 * them in `bin/{platform}-{arch}/`.
 *
 * Safety:
 *   - Concurrent-install safe: writes to a unique `.tmp-<rand>` file and
 *     atomically renames on success. Two processes may download in
 *     parallel; the last rename wins without torn bytes.
 *   - Integrity-checked: fetches the matching `<asset>.sha256` from the
 *     release and verifies the downloaded bytes before moving into
 *     place. A mismatched hash aborts the install and cleans up the
 *     temp file.
 *   - Crash-safe: failure mid-write leaves no partial binary behind —
 *     the temp file is removed in a `finally`, and the destination is
 *     only touched via `rename` after verification.
 */

import { chmodSync, existsSync, mkdirSync, renameSync, rmSync } from "node:fs";
import { join } from "node:path";

const pkg = (await Bun.file(join(import.meta.dir, "package.json")).json()) as {
  version: string;
  repository: { url: string };
};

const VERSION = pkg.version;
const REPO_URL = pkg.repository.url.replace(/^git\+/, "").replace(/\.git$/, "");
const TAG = `v${VERSION}`;
const BASE_URL = `${REPO_URL}/releases/download/${TAG}`;

type Platform = "windows" | "macos" | "linux";
type Arch = "x64" | "arm64";

function getPlatform(): Platform {
  switch (process.platform) {
    case "win32":
      return "windows";
    case "darwin":
      return "macos";
    default:
      return "linux";
  }
}

function getArch(): Arch {
  return process.arch === "arm64" ? "arm64" : "x64";
}

const platform = getPlatform();
const arch = getArch();
const ext = platform === "windows" ? ".exe" : "";
const platArch = `${platform}-${arch}`;

const binDir = join(import.meta.dir, "bin", platArch);
mkdirSync(binDir, { recursive: true });

async function fetchSha256(url: string): Promise<string> {
  const res = await fetch(`${url}.sha256`);
  if (!res.ok) {
    throw new Error(`missing checksum file ${url}.sha256: HTTP ${res.status}`);
  }
  // Release checksum files are either `<hash>` or `<hash>  <filename>` — take the first token.
  const text = (await res.text()).trim().split(/\s+/)[0];
  if (!text || !/^[0-9a-f]{64}$/i.test(text)) {
    throw new Error(`malformed checksum file at ${url}.sha256`);
  }
  return text.toLowerCase();
}

async function sha256(bytes: ArrayBuffer): Promise<string> {
  const hash = new Bun.CryptoHasher("sha256");
  hash.update(bytes);
  return hash.digest("hex");
}

async function download(runtime: "full" | "lite"): Promise<void> {
  const assetName = `tynd-${runtime}-${platArch}${ext}`;
  const destPath = join(binDir, `tynd-${runtime}${ext}`);

  if (existsSync(destPath)) {
    return;
  }

  const url = `${BASE_URL}/${assetName}`;

  const [expected, res] = await Promise.all([fetchSha256(url), fetch(url)]);
  if (!res.ok) {
    throw new Error(`Failed to download ${url}: HTTP ${res.status}`);
  }

  const buffer = await res.arrayBuffer();
  const actual = await sha256(buffer);
  if (actual !== expected) {
    throw new Error(`Checksum mismatch for ${assetName}: expected ${expected}, got ${actual}`);
  }

  // Write to a unique temp file, then atomically rename — safe under
  // concurrent postinstall from a monorepo. `.tmp-<rand>` also avoids
  // collisions with the destination when two processes race.
  const tempPath = `${destPath}.tmp-${process.pid}-${Date.now().toString(36)}`;
  try {
    await Bun.write(tempPath, buffer);
    if (platform !== "windows") {
      chmodSync(tempPath, 0o755);
    }
    renameSync(tempPath, destPath);
  } catch (e) {
    rmSync(tempPath, { force: true });
    throw e;
  }
}

// In a monorepo workspace the script runs from packages/host/ directly —
// no download needed (the dev is expected to `cargo build`).
if (!import.meta.dir.includes("node_modules")) process.exit(0);

try {
  await download("full");
  await download("lite");
} catch (_err) {
  process.exit(1);
}
