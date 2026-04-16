import { existsSync, mkdirSync, watch } from "node:fs"
import path from "node:path"
import { buildFrontendEntry, buildLiteBundle } from "../lib/bundle.ts"
import { hashSources, readCache, writeCache } from "../lib/cache.ts"
import { loadConfig, resolvePaths } from "../lib/config.ts"
import { detectFrontend, findBinary } from "../lib/detect.ts"
import { log } from "../lib/logger.ts"

export interface DevOptions {
  cwd: string
}

export async function dev(opts: DevOptions): Promise<void> {
  const cfg = resolvePaths(await loadConfig(opts.cwd), opts.cwd)
  const cacheDir = path.join(opts.cwd, ".vorn", "cache")

  // ── SSR check ────────────────────────────────────────────────────────────

  const frontend = await detectFrontend(opts.cwd)

  if (frontend.blockedBy) {
    log.error(`${frontend.blockedBy} detected — incompatible with server-side frameworks.`)
    log.dim("  Requires a pure SPA (React, Vue, Svelte, Angular, Solid, Lit, Preact…)")
    process.exit(1)
  }

  log.blank()
  log.info(`Starting in ${log.cyan("dev")} mode (${cfg.runtime})`)
  log.blank()

  // ── Host binary ───────────────────────────────────────────────────────────

  const binPath = findBinary(cfg.runtime, opts.cwd)
  if (!binPath) {
    log.error(`vorn-${cfg.runtime} binary not found.`)
    log.dim(`  Install: bun add @vorn/host`)
    process.exit(1)
  }

  // ── Frontend dev server ───────────────────────────────────────────────────

  const hasFwk = frontend.buildTool !== "none"
  const devUrl = cfg.devUrl ?? (hasFwk ? frontend.devUrl : null)
  const devCommand = cfg.devCommand ?? (hasFwk ? frontend.devCommand : null)

  let devServerProc: ReturnType<typeof Bun.spawn> | null = null

  if (devUrl && devCommand) {
    log.step(`Frontend: ${log.cyan(frontend.buildTool)} dev server → ${devUrl}`)
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
    log.success(`Dev server ready → ${devUrl}  ${log.dim("(HMR via Vite)")}`)
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
    log.step(`Frontend: static → ${log.gray(cfg.frontendDir)}`)
  }

  log.blank()

  // ── Backend bundle (lite) — initial build with cache ──────────────────────

  const backendSrcDir = path.dirname(cfg.backend)
  const bundlePath = path.join(cacheDir, "bundle.dev.js")

  if (cfg.runtime === "lite") {
    await buildBackendDev({ cfg, opts, cacheDir, bundlePath, backendSrcDir, silent: false })
    log.blank()
  }

  // ── Spawn the host binary ─────────────────────────────────────────────────

  const env: Record<string, string> = Object.fromEntries(
    Object.entries(process.env).filter(
      (entry): entry is [string, string] => entry[1] !== undefined,
    ),
  )
  if (devUrl) env["VORN_DEV_URL"] = devUrl
  // Full mode: backend reads these env vars — no need to hardcode in app.start()
  if (cfg.runtime === "full") {
    env["VORN_ENTRY"] = cfg.backend
    env["VORN_FRONTEND_DIR"] = cfg.frontendDir
  }

  const makeArgs = (): string[] => {
    const args: string[] =
      cfg.runtime === "lite" ? ["--bundle", bundlePath] : ["--backend-entry", cfg.backend]

    // Lite mode: QuickJS can't read env vars, so pass frontend location as CLI args.
    // Full mode: Bun subprocess reads VORN_DEV_URL / VORN_FRONTEND_DIR from env.
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

  log.step(`Binary:  ${log.gray(binPath)}`)
  log.blank()

  let hostProc = Bun.spawn([binPath, ...makeArgs()], {
    cwd: opts.cwd,
    stdout: "pipe",
    stderr: "pipe",
    env,
  })
  pipeWithPrefix(hostProc.stdout as ReadableStream<Uint8Array>, process.stdout, log.gray("[host]"))
  pipeWithPrefix(hostProc.stderr as ReadableStream<Uint8Array>, process.stderr, log.gray("[host]"))

  // ── Backend file watcher → restart on change ──────────────────────────────

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
      log.blank()
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

      hostProc = Bun.spawn([binPath, ...makeArgs()], {
        cwd: opts.cwd,
        stdout: "pipe",
        stderr: "pipe",
        env,
      })
      pipeWithPrefix(hostProc.stdout, process.stdout, log.gray("[host]"))
      pipeWithPrefix(hostProc.stderr, process.stderr, log.gray("[host]"))
      log.success(`${reason === "config" ? "Restarted" : "Reloaded"} in ${Date.now() - t0}ms`)
      log.blank()
      reloading = false
      if (reloadPending) {
        reloadPending = false
        triggerReload("backend")
      }
    }, 300)
  }

  const configPath = path.join(opts.cwd, "vorn.config.ts")
  const backendWatcher = watch(backendSrcDir, { recursive: true }, (_, filename) => {
    if (!filename || !WATCH_EXTS.test(filename)) return
    triggerReload("backend")
  })
  const configWatcher = existsSync(configPath)
    ? watch(configPath, () => triggerReload("config"))
    : null

  log.dim(
    `  Watching backend for changes… ${log.gray(`(${path.relative(opts.cwd, backendSrcDir)}/)`)}`,
  )
  log.dim(`  Watching ${log.gray("vorn.config.ts")} for changes…`)
  if (devUrl) log.dim(`  Frontend HMR active via ${frontend.buildTool}`)
  log.blank()

  // ── Graceful shutdown ─────────────────────────────────────────────────────

  const shutdown = () => {
    backendWatcher.close()
    configWatcher?.close()
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
  devServerProc?.kill()

  if (code !== 0 && code !== null) {
    log.error(`Process exited with code ${code}`)
    process.exit(code)
  }
}

// ── Backend bundle helper (dev mode — non-minified) ───────────────────────────

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
    [path.join(o.opts.cwd, "vorn.config.ts"), path.join(o.opts.cwd, "package.json")],
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

// ── Helpers ───────────────────────────────────────────────────────────────────

// Forward a child stream to our own, prefixing each line. Line-buffered so prefix
// attaches once per line even when chunks split mid-line.
function pipeWithPrefix(
  src: ReadableStream<Uint8Array>,
  dest: NodeJS.WriteStream,
  prefix: string,
): void {
  const decoder = new TextDecoder()
  let buf = ""
  ;(async () => {
    const reader = src.getReader()
    try {
      while (true) {
        const { done, value } = await reader.read()
        if (done) break
        buf += decoder.decode(value, { stream: true })
        let idx = buf.indexOf("\n")
        while (idx !== -1) {
          dest.write(`${prefix} ${buf.slice(0, idx)}\n`)
          buf = buf.slice(idx + 1)
          idx = buf.indexOf("\n")
        }
      }
      if (buf) dest.write(`${prefix} ${buf}\n`)
    } catch {
      /* stream closed mid-read — child exited */
    }
  })()
}

async function waitForServer(url: string, timeout = 30_000): Promise<boolean> {
  const start = Date.now()
  while (Date.now() - start < timeout) {
    try {
      const res = await fetch(url, { signal: AbortSignal.timeout(1000) })
      if (res.ok || res.status < 500) return true
    } catch {
      /* not ready yet */
    }
    await Bun.sleep(500)
  }
  return false
}
