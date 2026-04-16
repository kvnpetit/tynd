import { existsSync } from "node:fs"
import path from "node:path"
import { loadConfig, resolvePaths, type VornConfig } from "../lib/config.ts"
import { detectFrontend, findBinary } from "../lib/detect.ts"
import { log } from "../lib/logger.ts"

type Level = "error" | "warn" | "info"
interface Issue {
  level: Level
  message: string
  hint?: string | undefined
}

export interface ValidateOptions {
  cwd: string
  json: boolean
}

export async function validate(opts: ValidateOptions): Promise<void> {
  const issues: Issue[] = []

  const pass = (msg: string) => issues.push({ level: "info", message: msg })
  const warn = (msg: string, hint?: string) => issues.push({ level: "warn", message: msg, hint })
  const error = (msg: string, hint?: string) => issues.push({ level: "error", message: msg, hint })

  // 1. Config loads
  let cfg: VornConfig
  try {
    cfg = resolvePaths(await loadConfig(opts.cwd), opts.cwd)
    pass("vorn.config.ts valid")
  } catch {
    error("vorn.config.ts missing or invalid", "Run: vorn init")
    output(issues, opts)
    return
  }

  // 1b. Check for blocked SSR frameworks
  const frontend = await detectFrontend(opts.cwd)
  if (frontend.blockedBy) {
    error(
      `${frontend.blockedBy} detected — incompatible server-side framework`,
      "Requires a pure SPA (React, Vue, Svelte, Angular, Solid, Lit, Preact…)",
    )
    output(issues, opts)
    return
  }

  // 2. Backend entry
  if (existsSync(cfg.backend)) {
    pass(`backend entry exists`)
  } else {
    error(`backend entry not found`, `Expected: ${path.relative(opts.cwd, cfg.backend)}`)
  }

  // 3. Frontend directory
  if (existsSync(cfg.frontendDir)) {
    pass("frontend directory exists")
  } else {
    warn("frontend directory missing", "Run: vorn build")
  }

  // 4. Host binary
  const binPath = findBinary(cfg.runtime, opts.cwd)
  if (binPath) {
    pass(`vorn-${cfg.runtime} binary found`)
  } else {
    warn(`vorn-${cfg.runtime} binary not found`, `Install: bun add @vorn/${cfg.runtime}`)
  }

  output(issues, opts)
}

function output(issues: Issue[], opts: ValidateOptions): void {
  const errors = issues.filter((i) => i.level === "error").length
  const warns = issues.filter((i) => i.level === "warn").length

  if (opts.json) {
    console.log(JSON.stringify({ ok: errors === 0, issues }, null, 2))
    if (errors > 0) process.exit(1)
    return
  }

  log.blank()
  log.info(log.bold("vorn validate"))
  log.blank()

  for (const issue of issues) {
    if (issue.level === "error") {
      log.error(issue.message)
    } else if (issue.level === "warn") {
      log.warn(issue.message)
    } else {
      log.success(issue.message)
    }
    if (issue.hint) log.dim(`           → ${issue.hint}`)
  }

  log.blank()

  if (errors > 0) {
    log.error(`${errors} error(s), ${warns} warning(s)`)
    log.blank()
    process.exit(1)
  } else if (warns > 0) {
    log.warn(`${warns} warning(s)`)
  } else {
    log.success("All checks passed.")
  }

  log.blank()
}
