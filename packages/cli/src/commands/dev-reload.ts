import { existsSync, watch } from "node:fs"
import path from "node:path"
import { log } from "../lib/logger.ts"

const WATCH_EXTS = /\.(ts|tsx|mts|cts|js|jsx|mjs|cjs|json)$/

export type ReloadReason = "backend" | "config"

export interface ReloadDeps {
  runtime: "lite" | "full"
  cwd: string
  backendSrcDir: string
  /** Full restart: kill host, (re)build bundle, respawn. */
  fullRestart: () => Promise<void>
  /** Hot reload: write "reload\n" to host stdin. Returns false if stdin is closed. */
  hotReload: () => Promise<boolean>
  /** Rebuild lite dev bundle silently (hot-reload path). No-op in full. */
  rebuildBundle: () => Promise<void>
}

export interface ReloadController {
  close(): void
}

/** Wire backend / config / package.json watchers to a debounced reload queue. */
export function installWatchers(deps: ReloadDeps): ReloadController {
  let reloadTimer: ReturnType<typeof setTimeout> | null = null
  let reloading = false
  let reloadPending = false

  const run = async (reason: ReloadReason) => {
    reloading = true
    const t0 = Date.now()

    if (reason === "config") {
      log.info("Config changed — restarting…")
      await deps.fullRestart()
      log.success(`Restarted in ${Date.now() - t0}ms`)
    } else {
      log.info("Backend changed — hot reloading…")
      try {
        await deps.rebuildBundle()
      } catch (err) {
        log.error(`Bundle failed: ${err instanceof Error ? err.message : String(err)}`)
        return
      }
      const ok = await deps.hotReload()
      if (ok) {
        log.success(`Hot reloaded in ${Date.now() - t0}ms  ${log.dim("(window preserved)")}`)
      } else {
        log.warn("Hot reload channel closed — full restart")
        await deps.fullRestart()
        log.success(`Restarted in ${Date.now() - t0}ms`)
      }
    }
  }

  const trigger = (reason: ReloadReason) => {
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
      try {
        await run(reason)
      } finally {
        reloading = false
        if (reloadPending) {
          reloadPending = false
          trigger("backend")
        }
      }
    }, 300)
  }

  const backendWatcher = watch(deps.backendSrcDir, { recursive: true }, (_, filename) => {
    if (!filename || !WATCH_EXTS.test(filename)) return
    log.debug(`backend file changed: ${filename}`)
    trigger("backend")
  })

  const configPath = path.join(deps.cwd, "tynd.config.ts")
  const pkgPath = path.join(deps.cwd, "package.json")
  const configWatcher = existsSync(configPath) ? watch(configPath, () => trigger("config")) : null
  const pkgWatcher = existsSync(pkgPath) ? watch(pkgPath, () => trigger("config")) : null

  return {
    close() {
      backendWatcher.close()
      configWatcher?.close()
      pkgWatcher?.close()
      if (reloadTimer) clearTimeout(reloadTimer)
    },
  }
}
