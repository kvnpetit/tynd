#!/usr/bin/env bun
/**
 * Postinstall script for @tynd/host.
 * Downloads the tynd-full and tynd-lite binaries for the current platform
 * from GitHub Releases and places them in bin/{platform}-{arch}/.
 */

import { chmodSync, existsSync, mkdirSync } from "node:fs"
import { join } from "node:path"

// ── Config ────────────────────────────────────────────────────────────────────

const pkg = (await Bun.file(join(import.meta.dir, "package.json")).json()) as {
  version: string
  repository: { url: string }
}

const VERSION = pkg.version
const REPO_URL = pkg.repository.url.replace(/^git\+/, "").replace(/\.git$/, "")
// Single unified release-please tag: v<VERSION>
const TAG = `v${VERSION}`
const BASE_URL = `${REPO_URL}/releases/download/${TAG}`

// ── Platform detection ────────────────────────────────────────────────────────

type Platform = "windows" | "macos" | "linux"
type Arch = "x64" | "arm64"

function getPlatform(): Platform {
  switch (process.platform) {
    case "win32":
      return "windows"
    case "darwin":
      return "macos"
    default:
      return "linux"
  }
}

function getArch(): Arch {
  return process.arch === "arm64" ? "arm64" : "x64"
}

const platform = getPlatform()
const arch = getArch()
const ext = platform === "windows" ? ".exe" : ""
const platArch = `${platform}-${arch}`

// ── Download ──────────────────────────────────────────────────────────────────

const binDir = join(import.meta.dir, "bin", platArch)
mkdirSync(binDir, { recursive: true })

async function download(runtime: "full" | "lite"): Promise<void> {
  const assetName = `tynd-${runtime}-${platArch}${ext}`
  const destPath = join(binDir, `tynd-${runtime}${ext}`)

  if (existsSync(destPath)) {
    console.log(`  ✓ tynd-${runtime} already present, skipping`)
    return
  }

  const url = `${BASE_URL}/${assetName}`
  console.log(`  ↓ Downloading tynd-${runtime} (${platArch})…`)

  const res = await fetch(url)
  if (!res.ok) {
    throw new Error(`Failed to download ${url}: HTTP ${res.status}`)
  }

  const buffer = await res.arrayBuffer()
  await Bun.write(destPath, buffer)

  if (platform !== "windows") {
    chmodSync(destPath, 0o755)
  }

  console.log(`  ✓ tynd-${runtime} ready`)
}

// ── Main ──────────────────────────────────────────────────────────────────────

// In monorepo workspace, script runs from packages/host/ directly — skip download
if (!import.meta.dir.includes("node_modules")) process.exit(0)

console.log(`\n@tynd/host ${VERSION} — installing binaries (${platArch})\n`)

try {
  await download("full")
  await download("lite")
  console.log()
} catch (err) {
  console.error(`\n[ERROR] @tynd/host postinstall failed:\n  ${err}`)
  console.error(`  You can retry with: bun run install.ts\n`)
  process.exit(1)
}
