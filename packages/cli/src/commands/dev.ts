import { existsSync, mkdirSync, watch } from "node:fs"
import path from "node:path"
import { buildFrontendEntry, buildLiteBundle } from "../lib/bundle.ts"
import { hashSources, readCache, wipeIfStaleVersion, writeCache } from "../lib/cache.ts"
import { loadConfig, resolvePaths } from "../lib/config.ts"
import { detectFrontend, findBinary } from "../lib/detect.ts"
import { log } from "../lib/logger.ts"
import { pipeWithPrefix, waitForServer } from "../lib/spawn-helpers.ts"

export interface DevOptions {
  cwd: string
}

export async function dev(opts: DevOptions): Promise<void> {
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
  log.info(`Starting in ${log.cyan("dev")} mode (${cfg.runtime})`)

  const binPath = findBinary(cfg.runtime, opts.cwd)
  if (!binPath) {
    log.hint(`tynd-${cfg.runtime} binary not found.`, "Install: bun add @tynd/host")
    process.exit(1)
  }

  const hasFwk = frontend.buildTool !== "none"
  const devUrl = cfg.devUrl ?? (hasFwk ? frontend.devUrl : null)
  const devCommand = cfg.devCommand ?? (hasFwk ? frontend.devCommand : null)

  let devServerProc: ReturnType<typeof Bun.spawn> | null = null

  if (devUrl && devCommand) {
    log.step(`Frontend: ${log.cyan(frontend.buildTool)} dev server -> ${devUrl}`)
    const devParts = devCommand.split(/\s+/).filter(Boolean)
    devServerProc = Bun.spawn(devParts, {
      cwd: opts.cwd,
      stdout: "pipe",
      stderr: "pipe",
      env: { ...process.env },
    })
    pipeWithPrefix(
      devServerProc.stdout as ReadableStream<Uint8Array>,
      process.stdout,
      log.cyan(`[${frontend.buildTool}]`),
    )
    pipeWithPrefix(
      devServerProc.stderr as ReadableStream<Uint8Array>,
      process.stderr,
      log.cyan(`[${frontend.buildTool}]`),
    )
    log.step("Waiting for dev server…")
    if (!(await waitForServer(devUrl))) {
      log.error(`Dev server did not respond at ${devUrl} within 30s`)
      devServerProc.kill()
      process.exit(1)
    }
    log.success(`Dev server ready -> ${devUrl}  ${log.dim("(HMR via Vite)")}`)
  } else if (cfg.frontendEntry) {
    const entry = path.resolve(opts.cwd, cfg.frontendEntry)
    const frontendHash = hashSources([path.dirname(entry)], [entry])
    const cached = readCache(cacheDir, "frontend")

    if (cached?.hash === frontendHash && existsSync(cfg.frontendDir)) {
      log.step(`Frontend ${log.cyan("↑ cache")}`)
    } else {
      log.step(`Building frontend entry ${log.gray(cfg.frontendEntry)}…`)
      await buildFrontendEntry(entry, cfg.frontendDir)
      writeCache(cacheDir, "frontend", { hash: frontendHash, updatedAt: Date.now() })
      log.success("Frontend ready")
    }
  } else {
    log.step(`Frontend: static -> ${log.gray(cfg.frontendDir)}`)
  }

  const backendSrcDir = path.dirname(cfg.backend)
  const bundlePath = path.join(cacheDir, "bundle.dev.js")

  if (cfg.runtime === "lite") {
    await buildBackendDev({ cfg, opts, cacheDir, bundlePath, backendSrcDir, silent: false })
  }

  const env: Record<string, string> = Object.fromEntries(
    Object.entries(process.env).filter(
      (entry): entry is [string, string] => entry[1] !== undefined,
    ),
  )
  if (devUrl) env["TYND_DEV_URL"] = devUrl
  // Full mode: backend reads these env vars — no need to hardcode in app.start()
  if (cfg.runtime === "full") {
    env["TYND_ENTRY"] = cfg.backend
    env["TYND_FRONTEND_DIR"] = cfg.frontendDir
  }

  const makeArgs = (): string[] => {
    const args: string[] =
      cfg.runtime === "lite" ? ["--bundle", bundlePath] : ["--backend-entry", cfg.backend]

    // Lite mode: QuickJS can't read env vars, so pass frontend location as CLI args.
    // Full mode: Bun subprocess reads TYND_DEV_URL / TYND_FRONTEND_DIR from env.
    if (cfg.runtime === "lite") {
      if (devUrl) {
        args.push("--dev-url", devUrl)
      } else {
        args.push("--frontend-dir", cfg.frontendDir)
      }
    }

    args.push("--debug", ...(cfg.binaryArgs ?? []))
    return args
  }

  log.step(`Binary: ${log.gray(binPath)}`)

  const spawnHost = (): ReturnType<typeof Bun.spawn> => {
    const proc = Bun.spawn([binPath, ...makeArgs()], {
      cwd: opts.cwd,
      stdin: "pipe",
      stdout: "pipe",
      stderr: "pipe",
      env,
    })
    pipeWithPrefix(proc.stdout as ReadableStream<Uint8Array>, process.stdout, log.gray("[host]"))
    pipeWithPrefix(proc.stderr as ReadableStream<Uint8Array>, process.stderr, log.gray("[host]"))
    return proc
  }

  log.debug(`spawning host: ${binPath} ${makeArgs().join(" ")}`)
  let hostProc = spawnHost()

  // Backend change -> hot reload via stdin admin command. Host keeps the WebView
  // alive and respawns only the backend runtime (Bun subprocess in full mode,
  // QuickJS thread in lite mode). Config changes still do a full host restart
  // so window settings re-apply.
  const canHotReload = true

  const WATCH_EXTS = /\.(ts|tsx|mts|cts|js|jsx|mjs|cjs|json)$/

  let reloadTimer: ReturnType<typeof setTimeout> | null = null
  let reloading = false
  let reloadPending = false

  const triggerReload = (reason: "backend" | "config") => {
    if (reloading) {
      reloadPending = true
      return
    }
    if (reloadTimer) clearTimeout(reloadTimer)
    reloadTimer = setTimeout(async () => {
      if (reloading) {
        reloadPending = true
        return
      }
      reloading = true
      const t0 = Date.now()

      const fullRestart = reason === "config" || !canHotReload

      if (fullRestart) {
        log.info(
          reason === "config" ? `Config changed — restarting…` : `Backend changed — reloading…`,
        )
        hostProc.kill()
        await hostProc.exited.catch(() => undefined)

        if (cfg.runtime === "lite") {
          try {
            await buildBackendDev({ cfg, opts, cacheDir, bundlePath, backendSrcDir, silent: false })
          } catch (err) {
            log.error(`Bundle failed: ${err instanceof Error ? err.message : String(err)}`)
            reloading = false
            if (reloadPending) {
              reloadPending = false
              triggerReload("backend")
            }
            return
          }
        }
        hostProc = spawnHost()
        log.success(`${reason === "config" ? "Restarted" : "Reloaded"} in ${Date.now() - t0}ms`)
      } else {
        // Hot reload: host + webview stay alive, only the backend runtime restarts.
        log.info("Backend changed — hot reloading…")
        // Lite mode: rebuild the bundle first so the host re-reads fresh code.
        if (cfg.runtime === "lite") {
          try {
            await buildBackendDev({ cfg, opts, cacheDir, bundlePath, backendSrcDir, silent: true })
          } catch (err) {
            log.error(`Bundle failed: ${err instanceof Error ? err.message : String(err)}`)
            reloading = false
            if (reloadPending) {
              reloadPending = false
              triggerReload("backend")
            }
            return
          }
        }
        const stdin = hostProc.stdin
        if (stdin && typeof stdin === "object" && "write" in stdin) {
          ;(stdin as { write: (s: string) => void }).write("reload\n")
        }
        log.success(`Hot reloaded in ${Date.now() - t0}ms  ${log.dim("(window preserved)")}`)
      }

      reloading = false
      if (reloadPending) {
        reloadPending = false
        triggerReload("backend")
      }
    }, 300)
  }

  const configPath = path.join(opts.cwd, "tynd.config.ts")
  const pkgPath = path.join(opts.cwd, "package.json")

  const backendWatcher = watch(backendSrcDir, { recursive: true }, (_, filename) => {
    if (!filename || !WATCH_EXTS.test(filename)) return
    log.debug(`backend file changed: ${filename}`)
    triggerReload("backend")
  })
  const configWatcher = existsSync(configPath)
    ? watch(configPath, () => triggerReload("config"))
    : null
  // package.json changes affect deps / version / scripts — full restart.
  const pkgWatcher = existsSync(pkgPath) ? watch(pkgPath, () => triggerReload("config")) : null

  const watchTargets = [
    log.gray(`${path.relative(opts.cwd, backendSrcDir)}/`),
    log.gray("tynd.config.ts"),
    log.gray("package.json"),
    ...(devUrl ? [`frontend HMR (${frontend.buildTool})`] : []),
  ]
  log.step(`Watching: ${watchTargets.join(", ")}`)
  log.blank()

  const shutdown = () => {
    backendWatcher.close()
    configWatcher?.close()
    pkgWatcher?.close()
    if (reloadTimer) clearTimeout(reloadTimer)
    hostProc.kill()
    devServerProc?.kill()
    Promise.allSettled([
      hostProc.exited.catch(() => undefined),
      devServerProc ? devServerProc.exited.catch(() => undefined) : Promise.resolve(),
    ]).finally(() => process.exit(0))
  }

  process.on("SIGINT", shutdown)
  process.on("SIGTERM", shutdown)

  // Keep process alive until the host exits (e.g. user closes the window)
  const code = await hostProc.exited
  backendWatcher.close()
  configWatcher?.close()
  pkgWatcher?.close()
  devServerProc?.kill()

  if (code !== 0 && code !== null) {
    log.error(`Process exited with code ${code}`)
    process.exit(code)
  }
}

async function buildBackendDev(o: {
  cfg: ReturnType<typeof resolvePaths>
  opts: DevOptions
  cacheDir: string
  bundlePath: string
  backendSrcDir: string
  silent: boolean
}): Promise<void> {
  const backendHash = hashSources(
    [o.backendSrcDir],
    [path.join(o.opts.cwd, "tynd.config.ts"), path.join(o.opts.cwd, "package.json")],
  )
  const cached = readCache(o.cacheDir, "backend-dev")
  const cacheHit = cached?.hash === backendHash && existsSync(o.bundlePath)

  if (cacheHit) {
    if (!o.silent) log.step(`Backend  ${log.cyan("↑ cache")}`)
    return
  }

  if (!o.silent) log.step("Building backend bundle…")
  mkdirSync(o.cacheDir, { recursive: true })
  await buildLiteBundle(o.cfg.backend, o.bundlePath, false, true)
  writeCache(o.cacheDir, "backend-dev", { hash: backendHash, updatedAt: Date.now() })
  if (!o.silent) log.success("Bundle ready")
}

export { pipeWithPrefix, waitForServer } from "../lib/spawn-helpers.ts"
