import { loadConfig } from "../lib/config.ts"
import { findBinary, getArch, getPlatform } from "../lib/detect.ts"
import { log } from "../lib/logger.ts"
import { VERSION } from "../lib/version.ts"

export interface InfoOptions {
  cwd: string
  json: boolean
}

interface InfoReport {
  cli: { version: string }
  platform: { os: "windows" | "macos" | "linux"; arch: "x64" | "arm64" }
  bun: { version: string }
  project: {
    found: boolean
    cwd: string
    runtime?: "full" | "lite"
    backend?: string
    frontendDir?: string
    binary?: { path: string } | { missing: true }
  }
}

export async function info(opts: InfoOptions): Promise<void> {
  const cfg = await loadConfig(opts.cwd).catch(() => null)
  const report: InfoReport = {
    cli: { version: VERSION },
    platform: { os: getPlatform(), arch: getArch() },
    bun: { version: Bun.version },
    project: { found: cfg !== null, cwd: opts.cwd },
  }

  if (cfg) {
    report.project.runtime = cfg.runtime
    report.project.backend = cfg.backend
    report.project.frontendDir = cfg.frontendDir
    const binPath = findBinary(cfg.runtime, opts.cwd)
    report.project.binary = binPath ? { path: binPath } : { missing: true }
  }

  if (opts.json) {
    console.log(JSON.stringify(report, null, 2))
    return
  }

  log.blank()
  log.info(log.bold("Tynd environment"))
  log.blank()
  log.step(`CLI:       ${log.cyan(report.cli.version)}`)
  log.step(`Platform:  ${log.cyan(report.platform.os)} / ${report.platform.arch}`)
  log.step(`Bun:       ${report.bun.version}`)
  log.blank()

  if (cfg) {
    log.step(`Mode:      ${log.cyan(cfg.runtime)}`)
    log.step(`Backend:   ${cfg.backend}`)
    log.step(`Frontend:  ${cfg.frontendDir}`)

    if (report.project.binary && "path" in report.project.binary) {
      log.step(`Binary:    ${log.gray(report.project.binary.path)}`)
    } else {
      log.step(`Binary:    ${log.yellow(`tynd-${cfg.runtime} not found`)}`)
      log.dim(`           Install: bun add @tynd/host`)
    }
  } else {
    log.step("No tynd.config.ts found in current directory")
  }

  log.blank()
}
