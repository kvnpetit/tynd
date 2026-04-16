import { existsSync, rmSync } from "node:fs"
import path from "node:path"
import { loadConfig } from "../lib/config.ts"
import { log } from "../lib/logger.ts"
import { confirm } from "../lib/prompt.ts"

export interface CleanOptions {
  cwd: string
  yes: boolean
  dryRun: boolean
}

// Build output dir names — only delete frontendDir when it matches these
// to avoid wiping user source files (e.g. a `frontend/` source directory).
const BUILD_OUTPUT_DIRS = new Set(["dist", "build", "out", ".next", ".nuxt", ".svelte-kit"])

export async function clean(opts: CleanOptions): Promise<void> {
  const cfg = await loadConfig(opts.cwd).catch(() => null)

  // Only include frontendDir if it looks like a build output, not user sources.
  const frontendDir = cfg?.frontendDir ?? "frontend"
  const frontendAbs = path.join(opts.cwd, frontendDir)
  const frontendBasename = path.basename(frontendDir)
  const includeFrontend = BUILD_OUTPUT_DIRS.has(frontendBasename)

  const targets = [
    ...(includeFrontend ? [frontendAbs] : []),
    path.join(opts.cwd, "release"),
    path.join(opts.cwd, ".tynd", "cache"),
  ].filter(existsSync)
  log.debug(`clean targets (${targets.length}): ${targets.join(", ")}`)

  log.blank()

  if (targets.length === 0) {
    log.info("Nothing to clean — no build artifacts found.")
    log.blank()
    return
  }

  log.info(opts.dryRun ? "Would delete:" : "Will delete:")
  for (const t of targets) {
    log.step(path.relative(opts.cwd, t))
  }
  log.blank()

  if (opts.dryRun) {
    log.info(log.gray("Dry run — nothing was deleted."))
    log.blank()
    return
  }

  if (!opts.yes) {
    const ok = await confirm("Proceed with deletion?")
    if (!ok) {
      log.warn("Cancelled.")
      log.blank()
      return
    }
  }

  for (const t of targets) {
    rmSync(t, { recursive: true, force: true })
    log.success(`Removed ${log.gray(path.relative(opts.cwd, t))}`)
  }

  log.blank()
}
