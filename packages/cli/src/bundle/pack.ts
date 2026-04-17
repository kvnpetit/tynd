import {
  chmodSync,
  lstatSync,
  mkdtempSync,
  readdirSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs"
import { tmpdir } from "node:os"
import path from "node:path"
import { promisify } from "node:util"
import { zstdCompress as zstdCompressCb, zstdCompressSync } from "node:zlib"

const zstdAsync = promisify(zstdCompressCb)

import { pngToIco, setWindowsExeIcon } from "../lib/icon.ts"
import { log } from "../lib/logger.ts"
import { patchPeSubsystem } from "../lib/pe.ts"

export interface PackOpts {
  hostBin: string
  backendBundle: string
  frontendFiles: Array<{ abs: string; rel: string }>
  outFile: string
  platform: "windows" | "macos" | "linux"
  iconPath: string | null
  sidecars?: Array<{ name: string; path: string }>
}

/**
 * Appends a TYNDPKG section to `tynd-lite`. Wire format:
 * `[file_count u32 LE]` then `[path_len u16 LE][path][data_len u32 LE][data]` per file,
 * then `[section_size u64 LE]` + `"TYNDPKG\0"` magic.
 */
export async function packageLite(o: PackOpts): Promise<void> {
  log.step("Packing assets into binary…")

  // bundle.js kept raw — QuickJS reads it without a decompress step.
  const packFiles: PackEntry[] = [
    { rel: "bundle.js", data: readFileSync(o.backendBundle) },
    ...o.frontendFiles.map((f) => ({ rel: `frontend/${f.rel}`, abs: f.abs })),
    ...(o.sidecars ?? []).map((s) => ({ rel: `sidecar/${s.name}`, abs: s.path })),
  ]

  const tmpIconPath = addIcon(packFiles, o)
  await writeBinary(o, packFiles)
  cleanupTmpIcon(tmpIconPath)

  const size = lstatSync(o.outFile).size / 1_048_576
  log.success(
    `Binary -> ${log.cyan(`release/${path.basename(o.outFile)}`)}  (${size.toFixed(1)} MB)`,
  )
}

/** zstd-compresses Bun, packs it + assets. Host decompresses to a cache dir on first run. */
export async function packageFull(o: PackOpts): Promise<void> {
  log.step("Compressing runtime (this may take a moment)…")
  const bunBin = process.execPath

  // Icon embedded into the Bun copy so Task Manager shows the app icon for the subprocess.
  let bunSrcPath = bunBin
  let tmpBunDir: string | null = null
  if (o.platform === "windows" && o.iconPath) {
    tmpBunDir = mkdtempSync(path.join(tmpdir(), "tynd-bun-"))
    const appName = path.basename(o.outFile, ".exe")
    bunSrcPath = path.join(tmpBunDir, `${appName}.exe`)
    writeFileSync(bunSrcPath, readFileSync(bunBin))
    await setWindowsExeIcon(bunSrcPath, o.iconPath, appName)
  }

  const bunBytes = readFileSync(bunSrcPath)
  const bunCompressed = Buffer.from(await zstdAsync(bunBytes))
  if (tmpBunDir)
    try {
      rmSync(tmpBunDir, { recursive: true })
    } catch {
      /* intentional: best-effort cleanup */
    }
  log.success(`Runtime compressed (${(bunCompressed.length / 1_048_576).toFixed(0)} MB)`)

  log.step("Packing assets…")

  // bun.version MUST be first — Rust reads it to determine cache path before bun.zst
  const packFiles: PackEntry[] = [
    { rel: "bun.version", data: Buffer.from(Bun.version, "utf8") },
    { rel: "bun.zst", data: bunCompressed },
    { rel: "bundle.js", data: readFileSync(o.backendBundle) }, // NOT auto-compressed
    ...o.frontendFiles.map((f) => ({ rel: `frontend/${f.rel}`, abs: f.abs })),
    ...(o.sidecars ?? []).map((s) => ({ rel: `sidecar/${s.name}`, abs: s.path })),
  ]

  const tmpIconPath = addIcon(packFiles, o)
  await writeBinary(o, packFiles)
  cleanupTmpIcon(tmpIconPath)

  const size = lstatSync(o.outFile).size / 1_048_576
  log.success(
    `Binary -> ${log.cyan(`release/${path.basename(o.outFile)}`)}  (~${size.toFixed(0)} MB)`,
  )
}

function addIcon(files: PackEntry[], o: PackOpts): string | null {
  if (!o.iconPath) return null
  const ext = path.extname(o.iconPath).toLowerCase()
  if (ext === ".png") {
    const icoBytes = pngToIco(readFileSync(o.iconPath))
    const tmp = path.join(path.dirname(o.outFile), ".icon-tmp.ico")
    writeFileSync(tmp, icoBytes)
    files.push({ rel: "icon.ico", data: readFileSync(tmp) })
    return tmp
  }
  files.push({ rel: "icon.ico", data: readFileSync(o.iconPath) })
  return null
}

async function writeBinary(o: PackOpts, files: PackEntry[]): Promise<void> {
  const packed = packAssets(files)
  const hostBytes = readFileSync(o.hostBin)
  const out = Buffer.concat([hostBytes, packed])

  writeFileSync(o.outFile, out)
  if (o.platform !== "windows") chmodSync(o.outFile, 0o755)

  // On Windows: flip PE subsystem to GUI + embed icon resources into the final .exe
  if (o.platform === "windows") {
    patchPeSubsystem(o.outFile)
    if (o.iconPath) await setWindowsExeIcon(o.outFile, o.iconPath)
  }
}

function cleanupTmpIcon(tmp: string | null): void {
  if (!tmp) return
  try {
    rmSync(tmp)
  } catch {
    /* intentional: best-effort cleanup */
  }
}

const TEXT_EXTS = /\.(html|htm|js|mjs|cjs|css|json|svg)$/i

// `{ rel, abs }` -> read from disk, text files auto-zstd'd with `.zst` suffix.
// `{ rel, data }` -> pre-processed buffer, packed as-is.
type PackEntry = { rel: string; abs: string } | { rel: string; data: Buffer }

function packAssets(files: PackEntry[]): Buffer {
  const chunks: Buffer[] = []
  let sectionSize = 0

  const countBuf = Buffer.allocUnsafe(4)
  countBuf.writeUInt32LE(files.length, 0)
  chunks.push(countBuf)
  sectionSize += 4

  for (const entry of files) {
    let packRel: string
    let data: Buffer

    if ("data" in entry) {
      packRel = entry.rel
      data = entry.data
    } else {
      const shouldCompress = TEXT_EXTS.test(entry.rel)
      packRel = shouldCompress ? `${entry.rel}.zst` : entry.rel
      let raw: Buffer
      try {
        raw = readFileSync(entry.abs)
      } catch (e) {
        throw new Error(`Failed to read asset "${entry.rel}": ${e}`)
      }
      data = shouldCompress ? Buffer.from(zstdCompressSync(raw)) : raw
    }

    const pathBytes = Buffer.from(packRel, "utf8")

    const pathLenBuf = Buffer.allocUnsafe(2)
    pathLenBuf.writeUInt16LE(pathBytes.length, 0)

    const dataLenBuf = Buffer.allocUnsafe(4)
    dataLenBuf.writeUInt32LE(data.length, 0)

    chunks.push(pathLenBuf, pathBytes, dataLenBuf, data)
    sectionSize += 2 + pathBytes.length + 4 + data.length
  }

  const trailer = Buffer.allocUnsafe(16)
  trailer.writeBigUInt64LE(BigInt(sectionSize), 0)
  trailer.write("TYNDPKG\0", 8, "ascii")
  chunks.push(trailer)

  return Buffer.concat(chunks)
}

export function collectFiles(dir: string): Array<{ abs: string; rel: string }> {
  const results: Array<{ abs: string; rel: string }> = []
  function walk(cur: string, prefix: string) {
    for (const entry of readdirSync(cur)) {
      const abs = path.join(cur, entry)
      const rel = prefix ? `${prefix}/${entry}` : entry
      // lstat first so symlinks don't get followed — prevents loops on self-referential dirs.
      const lst = lstatSync(abs)
      if (lst.isSymbolicLink()) continue
      if (lst.isDirectory()) {
        walk(abs, rel)
      } else {
        results.push({ abs, rel })
      }
    }
  }
  walk(dir, "")
  return results
}
