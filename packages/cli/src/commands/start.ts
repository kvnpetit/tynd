import { existsSync, mkdirSync } from "node:fs"
import path from "node:path"
import { buildFrontendEntry, buildLiteBundle } from "../lib/bundle.ts"
import { hashSources, readCache, wipeIfStaleVersion, writeCache } from "../lib/cache.ts"
import { loadConfig, resolvePaths } from "../lib/config.ts"
import { detectFrontend, findBinary } from "../lib/detect.ts"
import { exec } from "../lib/exec.ts"
import { log } from "../lib/logger.ts"
import { pipeWithPrefix } from "../lib/spawn-helpers.ts"
import { collectFiles } from "./build.ts"

export interface StartOptions {
  cwd: string
}

export async function start(opts: StartOptions): Promise<void> {
  const cfg = resolvePaths(await loadConfig(opts.cwd), opts.cwd)
  const cacheDir = path.join(opts.cwd, ".tynd", "cache")
  wipeIfStaleVersion(cacheDir)

  const frontend = await detectFrontend(opts.cwd)

  if (frontend.blockedBy) {
    log.hint(
      `${frontend.blockedBy} detected — incompatible with server-side frameworks.`,
      "Requires a pure SPA (React, Vue, Svelte, Angular, Solid, Lit, Preact…)",
    )
    process.exit(1)
  }

  log.blank()
  log.info(`Starting in ${log.cyan("start")} mode (${cfg.runtime})`)

  const binPath = findBinary(cfg.runtime, opts.cwd)
  if (!binPath) {
    log.hint(`tynd-${cfg.runtime} binary not found.`, "Install: bun add @tynd/host")
    process.exit(1)
  }

  // Frontend: production build, cached by source hash.
  if (frontend.buildTool !== "none" && frontend.buildCommand) {
    const frontendHash = hashSources(
      [path.join(opts.cwd, "src"), path.join(opts.cwd, "public")],
      [
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
        path.join(opts.cwd, ".env"),
        path.join(opts.cwd, ".env.production"),
        path.join(opts.cwd, ".env.local"),
      ],
    )
    const cached = readCache(cacheDir, "frontend")
    const outHasFiles = existsSync(cfg.frontendDir) && collectFiles(cfg.frontendDir).length > 0
    if (cached?.hash === frontendHash && outHasFiles) {
      log.step(`Frontend ${log.cyan("↑ cache")}`)
    } else {
      log.step(`Building frontend (${frontend.buildTool})…`)
      const parts = frontend.buildCommand.split(/\s+/).filter(Boolean)
      await exec(parts[0]!, parts.slice(1), { cwd: opts.cwd })
      writeCache(cacheDir, "frontend", { hash: frontendHash, updatedAt: Date.now() })
      log.success(`Frontend -> ${log.gray(frontend.outDir)}`)
    }
  } else if (cfg.frontendEntry) {
    const entry = path.resolve(opts.cwd, cfg.frontendEntry)
    const frontendHash = hashSources([path.dirname(entry)], [entry])
    const cached = readCache(cacheDir, "frontend")
    const outHasFiles = existsSync(cfg.frontendDir) && collectFiles(cfg.frontendDir).length > 0
    if (cached?.hash === frontendHash && outHasFiles) {
      log.step(`Frontend ${log.cyan("↑ cache")}`)
    } else {
      log.step(`Building frontend entry ${log.gray(cfg.frontendEntry)}…`)
      await buildFrontendEntry(entry, cfg.frontendDir)
      writeCache(cacheDir, "frontend", { hash: frontendHash, updatedAt: Date.now() })
      log.success(`Frontend -> ${log.gray(cfg.frontendDir)}`)
    }
  } else {
    log.step(`Frontend: static -> ${log.gray(cfg.frontendDir)}`)
  }

  // Backend (lite only): production bundle, cached by source hash.
  let bundlePath: string | null = null
  if (cfg.runtime === "lite") {
    bundlePath = path.join(cacheDir, "bundle.js")
    const backendHash = hashSources(
      [path.dirname(cfg.backend)],
      [path.join(opts.cwd, "tynd.config.ts"), path.join(opts.cwd, "package.json")],
    )
    const cached = readCache(cacheDir, "backend")
    if (cached?.hash === backendHash && existsSync(bundlePath)) {
      log.step(`Backend  ${log.cyan("↑ cache")}`)
    } else {
      mkdirSync(cacheDir, { recursive: true })
      log.step("Bundling backend (lite)…")
      await buildLiteBundle(cfg.backend, bundlePath, true)
      writeCache(cacheDir, "backend", { hash: backendHash, updatedAt: Date.now() })
      log.success("Backend bundled")
    }
  }

  const env: Record<string, string> = Object.fromEntries(
    Object.entries(process.env).filter(
      (entry): entry is [string, string] => entry[1] !== undefined,
    ),
  )
  if (cfg.runtime === "full") {
    env["TYND_ENTRY"] = cfg.backend
    env["TYND_FRONTEND_DIR"] = cfg.frontendDir
  }

  const args: string[] =
    cfg.runtime === "lite"
      ? ["--bundle", bundlePath!, "--frontend-dir", cfg.frontendDir]
      : ["--backend-entry", cfg.backend]

  args.push(...(cfg.binaryArgs ?? []))

  log.step(`Binary: ${log.gray(binPath)}`)
  log.blank()

  const hostProc = Bun.spawn([binPath, ...args], {
    cwd: opts.cwd,
    stdin: "pipe",
    stdout: "pipe",
    stderr: "pipe",
    env,
  })
  pipeWithPrefix(hostProc.stdout as ReadableStream<Uint8Array>, process.stdout, log.gray("[host]"))
  pipeWithPrefix(hostProc.stderr as ReadableStream<Uint8Array>, process.stderr, log.gray("[host]"))

  const shutdown = () => {
    hostProc.kill()
    hostProc.exited.catch(() => undefined).finally(() => process.exit(0))
  }

  process.on("SIGINT", shutdown)
  process.on("SIGTERM", shutdown)

  const code = await hostProc.exited

  if (code !== 0 && code !== null) {
    log.error(`Process exited with code ${code}`)
    process.exit(code)
  }
}
