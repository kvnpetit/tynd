import { log } from "../lib/logger.ts"
import { loadConfig } from "../lib/config.ts"
import { findBinary, getPlatform, getArch } from "../lib/detect.ts"

export async function info(cwd = process.cwd()): Promise<void> {
  log.blank()
  log.info(log.bold("Vorn environment"))
  log.blank()

  log.step(`Platform:  ${log.cyan(getPlatform())} / ${getArch()}`)
  log.step(`Bun:       ${Bun.version}`)

  log.blank()

  const cfg = await loadConfig(cwd).catch(() => null)
  if (cfg) {
    log.step(`Mode:      ${log.cyan(cfg.runtime)}`)
    log.step(`Backend:   ${cfg.backend}`)
    log.step(`Frontend:  ${cfg.frontendDir}`)

    const binPath = findBinary(cfg.runtime, cwd)
    if (binPath) {
      log.step(`Binary:    ${log.gray(binPath)}`)
    } else {
      log.step(`Binary:    ${log.yellow(`vorn-${cfg.runtime} not found`)}`)
      log.dim(`           Install: bun add @vorn/${cfg.runtime}`)
    }
  } else {
    log.step("No vorn.config.ts found in current directory")
  }

  log.blank()
}
