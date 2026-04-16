import {
  chmodSync,
  existsSync,
  lstatSync,
  mkdirSync,
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

import { buildFrontendEntry, buildFullBundle, buildLiteBundle } from "../lib/bundle.ts"
import { hashSources, readCache, writeCache } from "../lib/cache.ts"
import { loadConfig, resolvePaths } from "../lib/config.ts"
import { detectFrontend, findBinary, getPlatform } from "../lib/detect.ts"
import { exec } from "../lib/exec.ts"
import { detectIcon, pngToIco, setWindowsExeIcon } from "../lib/icon.ts"
import { log } from "../lib/logger.ts"
import { patchPeSubsystem } from "../lib/pe.ts"

export interface BuildOptions {
  cwd: string
  /** Output binary path. Defaults to release/<name>[.exe] */
  outfile?: string
}

export async function build(opts: BuildOptions): Promise<void> {
  const cfg = resolvePaths(await loadConfig(opts.cwd), opts.cwd)
  const cacheDir = path.join(opts.cwd, ".vorn", "cache")

  const frontend = await detectFrontend(opts.cwd)

  if (frontend.blockedBy) {
    log.hint(
      `${frontend.blockedBy} detected — incompatible with server-side frameworks.`,
      "Requires a pure SPA (React, Vue, Svelte, Angular, Solid, Lit, Preact…)",
    )
    process.exit(1)
  }

  log.blank()
  log.info(`Building ${log.cyan(cfg.runtime)} app`)
  log.debug(`backend=${cfg.backend} frontendDir=${cfg.frontendDir}`)
  log.blank()
  const t0 = Date.now()

  if (frontend.buildTool !== "none" && frontend.buildCommand) {
    const frontendHash = hashSources(
      [
        path.join(opts.cwd, "src"), // main source
        path.join(opts.cwd, "public"), // static assets
      ],
      [
        // Root config / entry files
        path.join(opts.cwd, "index.html"),
        path.join(opts.cwd, "vite.config.ts"),
        path.join(opts.cwd, "vite.config.js"),
        path.join(opts.cwd, "vite.config.mts"),
        path.join(opts.cwd, "vite.config.mjs"),
        path.join(opts.cwd, "svelte.config.ts"),
        path.join(opts.cwd, "svelte.config.js"),
        path.join(opts.cwd, "angular.json"),
        path.join(opts.cwd, "tsconfig.json"),
        path.join(opts.cwd, "tsconfig.app.json"),
        path.join(opts.cwd, "tsconfig.node.json"),
        path.join(opts.cwd, "package.json"),
        // Env files (affect production build output)
        path.join(opts.cwd, ".env"),
        path.join(opts.cwd, ".env.production"),
        path.join(opts.cwd, ".env.local"),
      ],
    )

    const cached = readCache(cacheDir, "frontend")
    const outHasFiles = existsSync(cfg.frontendDir) && collectFiles(cfg.frontendDir).length > 0
    const hit = cached?.hash === frontendHash && outHasFiles

    if (hit) {
      log.step(`Frontend ${log.cyan("↑ cache")}`)
    } else {
      log.step(`Building frontend (${frontend.buildTool})…`)
      const buildParts = frontend.buildCommand.split(/\s+/).filter(Boolean)
      await exec(buildParts[0]!, buildParts.slice(1), { cwd: opts.cwd })
      writeCache(cacheDir, "frontend", { hash: frontendHash, updatedAt: Date.now() })
      log.success(`Frontend → ${log.gray(frontend.outDir)}`)
    }
  } else if (cfg.frontendEntry) {
    const entry = path.resolve(opts.cwd, cfg.frontendEntry)
    const frontendHash = hashSources([path.dirname(entry)], [entry])
    const cached = readCache(cacheDir, "frontend")
    const outHasFiles = existsSync(cfg.frontendDir) && collectFiles(cfg.frontendDir).length > 0
    const hit = cached?.hash === frontendHash && outHasFiles

    if (hit) {
      log.step(`Frontend ${log.cyan("↑ cache")}`)
    } else {
      log.step(`Building frontend entry ${log.gray(cfg.frontendEntry)}…`)
      await buildFrontendEntry(entry, cfg.frontendDir)
      writeCache(cacheDir, "frontend", { hash: frontendHash, updatedAt: Date.now() })
      log.success(`Frontend → ${log.gray(cfg.frontendDir)}`)
    }
  } else {
    log.step("No frontend build step — skipping")
  }

  const backendSrcDir = path.dirname(cfg.backend)
  const bundleFilename = cfg.runtime === "lite" ? "bundle.js" : "bundle.dist.js"
  const cachedBundle = path.join(cacheDir, bundleFilename)

  const backendHash = hashSources(
    [backendSrcDir],
    [path.join(opts.cwd, "vorn.config.ts"), path.join(opts.cwd, "package.json")],
  )
  const backendCached = readCache(cacheDir, "backend")
  const backendHit = backendCached?.hash === backendHash && existsSync(cachedBundle)

  if (backendHit) {
    log.step(`Backend  ${log.cyan("↑ cache")}`)
  } else {
    mkdirSync(cacheDir, { recursive: true })
    if (cfg.runtime === "lite") {
      log.step("Bundling backend (lite)…")
      await buildLiteBundle(cfg.backend, cachedBundle, true)
    } else {
      log.step("Bundling backend (full)…")
      await buildFullBundle(cfg.backend, cachedBundle)
    }
    writeCache(cacheDir, "backend", { hash: backendHash, updatedAt: Date.now() })
    log.success("Backend bundled")
  }

  const hostBin = findBinary(cfg.runtime, opts.cwd)
  if (!hostBin) {
    log.hint(`vorn-${cfg.runtime} binary not found.`, "Install: bun add @vorn/host")
    process.exit(1)
  }
  log.step(`Host: ${log.gray(path.relative(opts.cwd, hostBin))}`)

  if (!existsSync(cfg.frontendDir)) {
    log.error(`Frontend directory not found: ${log.gray(cfg.frontendDir)}`)
    process.exit(1)
  }
  const frontendFiles = collectFiles(cfg.frontendDir)
  log.step(`Frontend: ${frontendFiles.length} files`)

  const iconPath = await detectIcon(opts.cwd, cfg.icon)
  if (iconPath) {
    log.step(`Icon:    ${log.gray(path.relative(opts.cwd, iconPath))}`)
  }

  const platform = getPlatform()
  const binExt = platform === "windows" ? ".exe" : ""
  const appName = path.basename(opts.cwd)
  const outFile = opts.outfile
    ? path.resolve(opts.cwd, opts.outfile)
    : path.join(opts.cwd, "release", appName + binExt)

  mkdirSync(path.dirname(outFile), { recursive: true })
  log.debug(`out=${outFile} host=${hostBin}`)

  if (cfg.runtime === "lite") {
    await packageLite({
      hostBin,
      backendBundle: cachedBundle,
      frontendFiles,
      outFile,
      platform,
      iconPath,
    })
  } else {
    await packageFull({
      hostBin,
      backendBundle: cachedBundle,
      frontendFiles,
      outFile,
      platform,
      iconPath,
    })
  }
  log.debug(`build finished in ${Date.now() - t0}ms`)
}

//
// Appends a packed section directly to the vorn-lite binary.
// No Bun runtime — the output is purely Rust + QuickJS.
//
// Format (appended after binary):
//   [file_count: u32 LE]
//   per file: [path_len: u16 LE][path: UTF-8][data_len: u32 LE][data: bytes]
//   [section_size: u64 LE]  ← total bytes above (file_count + all file entries)
//   [magic: "VORNPKG\0" 8 bytes]

interface LitePackOpts {
  hostBin: string
  backendBundle: string
  frontendFiles: Array<{ abs: string; rel: string }>
  outFile: string
  platform: "windows" | "macos" | "linux"
  iconPath: string | null
}

async function packageLite(o: LitePackOpts): Promise<void> {
  log.step("Packing assets into binary…")

  // bundle.js: data entry — never auto-compressed (QuickJS reads it directly)
  const packFiles: PackEntry[] = [
    { rel: "bundle.js", data: readFileSync(o.backendBundle) },
    ...o.frontendFiles.map((f) => ({ rel: `frontend/${f.rel}`, abs: f.abs })),
  ]

  // Pack icon — stored as icon.ico (converted from PNG if needed)
  let tmpIconPath: string | null = null
  if (o.iconPath) {
    const ext = path.extname(o.iconPath).toLowerCase()
    if (ext === ".png") {
      const icoBytes = pngToIco(readFileSync(o.iconPath))
      tmpIconPath = path.join(path.dirname(o.outFile), ".icon-tmp.ico")
      writeFileSync(tmpIconPath, icoBytes)
      packFiles.push({ rel: "icon.ico", data: readFileSync(tmpIconPath) })
    } else {
      packFiles.push({ rel: "icon.ico", data: readFileSync(o.iconPath) })
    }
  }

  const packed = packAssets(packFiles)
  const hostBytes = readFileSync(o.hostBin)
  const out = Buffer.concat([hostBytes, packed])

  writeFileSync(o.outFile, out)
  if (o.platform !== "windows") chmodSync(o.outFile, 0o755)

  // Clean up temp icon
  if (tmpIconPath)
    try {
      rmSync(tmpIconPath)
    } catch {
      /* intentional: best-effort cleanup */
    }

  // On Windows: patch PE subsystem (console → GUI) and embed icon resources
  if (o.platform === "windows") {
    patchPeSubsystem(o.outFile)
    if (o.iconPath) await setWindowsExeIcon(o.outFile, o.iconPath)
  }

  const sizeMb = (out.length / 1_048_576).toFixed(1)
  log.blank()
  log.success(`Binary → ${log.cyan(`release/${path.basename(o.outFile)}`)}  (${sizeMb} MB)`)
  log.blank()
}

//
// Compresses the local Bun binary with zstd and packs it into the vorn-full
// host binary using the VORNPKG\0 format (same as lite mode).
// At runtime, Rust decompresses Bun once to a persistent cache dir.

interface FullPackOpts {
  hostBin: string
  backendBundle: string
  frontendFiles: Array<{ abs: string; rel: string }>
  outFile: string
  platform: "windows" | "macos" | "linux"
  iconPath: string | null
}

async function packageFull(o: FullPackOpts): Promise<void> {
  log.step("Compressing runtime (this may take a moment)…")
  const bunBin = process.execPath

  // On Windows: embed app icon into the bun binary copy before compression,
  // so the extracted subprocess shows the app icon in Task Manager.
  let bunSrcPath = bunBin
  let tmpBunDir: string | null = null
  if (o.platform === "windows" && o.iconPath) {
    tmpBunDir = mkdtempSync(path.join(tmpdir(), "vorn-bun-"))
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
  ]

  // Pack icon — convert PNG → ICO if needed
  let tmpIconPath: string | null = null
  if (o.iconPath) {
    const ext = path.extname(o.iconPath).toLowerCase()
    if (ext === ".png") {
      const icoBytes = pngToIco(readFileSync(o.iconPath))
      tmpIconPath = path.join(path.dirname(o.outFile), ".icon-tmp.ico")
      writeFileSync(tmpIconPath, icoBytes)
      packFiles.push({ rel: "icon.ico", data: readFileSync(tmpIconPath) })
    } else {
      packFiles.push({ rel: "icon.ico", data: readFileSync(o.iconPath) })
    }
  }

  const packed = packAssets(packFiles)
  const hostBytes = readFileSync(o.hostBin)
  const out = Buffer.concat([hostBytes, packed])

  writeFileSync(o.outFile, out)
  if (o.platform !== "windows") chmodSync(o.outFile, 0o755)

  // Clean up temp icon
  if (tmpIconPath)
    try {
      rmSync(tmpIconPath)
    } catch {
      /* intentional: best-effort cleanup */
    }

  // On Windows: patch PE subsystem (console → GUI) and embed icon resources
  if (o.platform === "windows") {
    patchPeSubsystem(o.outFile)
    if (o.iconPath) await setWindowsExeIcon(o.outFile, o.iconPath)
  }

  const sizeMb = (out.length / 1_048_576).toFixed(0)
  log.blank()
  log.success(`Binary → ${log.cyan(`release/${path.basename(o.outFile)}`)}  (~${sizeMb} MB)`)
  log.blank()
}

const TEXT_EXTS = /\.(html|htm|js|mjs|cjs|css|json|svg)$/i

/**
 * A pack entry is either:
 *  - `{ rel, abs }` — read from disk, auto-compress text files (TEXT_EXTS) with zstd and append .zst suffix
 *  - `{ rel, data }` — pre-processed buffer, packed as-is (rel and data are already final)
 */
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
      // Pre-processed: use rel and data as-is
      packRel = entry.rel
      data = entry.data
    } else {
      // File on disk: auto-compress text files with zstd
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
  trailer.write("VORNPKG\0", 8, "ascii")
  chunks.push(trailer)

  return Buffer.concat(chunks)
}

function collectFiles(dir: string): Array<{ abs: string; rel: string }> {
  const results: Array<{ abs: string; rel: string }> = []
  function walk(cur: string, prefix: string) {
    for (const entry of readdirSync(cur)) {
      const abs = path.join(cur, entry)
      const rel = prefix ? `${prefix}/${entry}` : entry
      // Use lstatSync to detect symlinks before following them — prevents
      // infinite loops if a symlink points to a parent directory.
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
