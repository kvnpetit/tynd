import { existsSync, mkdirSync } from "node:fs"
import path from "node:path"

import { buildBundleContext } from "../bundle/context.ts"
import { parseBundleFlag, resolveTargets, runBundle } from "../bundle/index.ts"
import { collectFiles, packageFull, packageLite } from "../bundle/pack.ts"
import { signMacos, signWindows } from "../bundle/sign.ts"
import type { BundleTarget } from "../bundle/types.ts"
import { buildFrontendEntry, buildFullBundle, buildLiteBundle } from "../lib/bundle.ts"
import { hashSources, readCache, wipeIfStaleVersion, writeCache } from "../lib/cache.ts"
import { loadConfig, resolvePaths } from "../lib/config.ts"
import { detectFrontend, findBinary, getPlatform } from "../lib/detect.ts"
import { exec } from "../lib/exec.ts"
import { detectIcon } from "../lib/icon.ts"
import { log } from "../lib/logger.ts"

export { collectFiles }

export interface BuildOptions {
  cwd: string
  /** Output binary path. Defaults to release/<name>[.exe] */
  outfile?: string
  bundle?: string | boolean
}

export async function build(opts: BuildOptions): Promise<void> {
  const cfg = resolvePaths(await loadConfig(opts.cwd), opts.cwd)
  // Absolute path — Bun's mkdirSync recursive has a Windows bug with ../ paths.
  const cacheDir = path.resolve(opts.cwd, ".tynd", "cache")
  wipeIfStaleVersion(cacheDir)

  // Validate --bundle before doing any work so config errors surface fast.
  let bundleTargets: readonly BundleTarget[] = []
  try {
    const bundleRequested = parseBundleFlag(opts.bundle)
    if (bundleRequested && !cfg.bundle) {
      log.hint(
        "--bundle was passed but tynd.config.ts has no `bundle` block.",
        'Add: bundle: { identifier: "com.example.myapp" }',
      )
      process.exit(1)
    }
    if (bundleRequested) bundleTargets = resolveTargets(bundleRequested, getPlatform())
  } catch (e) {
    log.error(e instanceof Error ? e.message : String(e))
    process.exit(1)
  }

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
      log.success(`Frontend -> ${log.gray(frontend.outDir)}`)
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
      log.success(`Frontend -> ${log.gray(cfg.frontendDir)}`)
    }
  } else {
    log.step("No frontend build step — skipping")
  }

  const backendSrcDir = path.dirname(cfg.backend)
  const bundleFilename = cfg.runtime === "lite" ? "bundle.js" : "bundle.dist.js"
  const cachedBundle = path.join(cacheDir, bundleFilename)

  const backendHash = hashSources(
    [backendSrcDir],
    [path.join(opts.cwd, "tynd.config.ts"), path.join(opts.cwd, "package.json")],
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
    log.hint(`tynd-${cfg.runtime} binary not found.`, "Install: bun add @tynd/host")
    process.exit(1)
  }
  log.step(`Host: ${log.gray(path.relative(opts.cwd, hostBin))}`)

  if (!existsSync(cfg.frontendDir)) {
    log.error(`Frontend directory not found: ${log.gray(cfg.frontendDir)}`)
    process.exit(1)
  }
  const frontendFiles = collectFiles(cfg.frontendDir)
  log.step(`Frontend: ${frontendFiles.length} files`)

  const iconPath = detectIcon(opts.cwd, cfg.icon)
  if (iconPath) {
    log.step(`Icon:    ${log.gray(path.relative(opts.cwd, iconPath))}`)
  }

  const platform = getPlatform()
  const binExt = platform === "windows" ? ".exe" : ""
  const appName = path.basename(opts.cwd)
  const outFile = opts.outfile
    ? path.resolve(opts.cwd, opts.outfile)
    : path.resolve(opts.cwd, "release", appName + binExt)

  mkdirSync(path.dirname(outFile), { recursive: true })
  log.debug(`out=${outFile} host=${hostBin}`)

  const sidecars = cfg.sidecars ?? []
  if (sidecars.length > 0) {
    for (const s of sidecars) {
      if (!existsSync(s.path)) {
        log.error(`Sidecar '${s.name}' not found at ${s.path}`)
        process.exit(1)
      }
    }
    log.step(`Sidecars: ${sidecars.map((s) => s.name).join(", ")}`)
  }

  if (cfg.runtime === "lite") {
    await packageLite({
      hostBin,
      backendBundle: cachedBundle,
      frontendFiles,
      outFile,
      platform,
      iconPath,
      sidecars,
    })
  } else {
    await packageFull({
      hostBin,
      backendBundle: cachedBundle,
      frontendFiles,
      outFile,
      platform,
      iconPath,
      sidecars,
    })
  }

  // Sign the raw binary before any bundle step copies it into installers.
  // Gatekeeper / SmartScreen reject unsigned binaries post-install anyway,
  // so signing upstream of bundlers keeps every downstream artifact valid.
  if (platform === "windows") {
    await signWindows(cfg.bundle, outFile)
  } else if (platform === "macos") {
    await signMacos(cfg.bundle, outFile)
  }

  if (bundleTargets.length > 0) {
    const ctx = await buildBundleContext({
      cwd: opts.cwd,
      cfg,
      inputBinary: outFile,
      outDir: path.dirname(outFile),
      iconSource: iconPath,
    })
    await runBundle(ctx, bundleTargets)
  }

  log.debug(`build finished in ${Date.now() - t0}ms`)
}
