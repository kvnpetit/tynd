import { existsSync } from "node:fs"
import path from "node:path"
import { loadConfig, resolvePaths } from "../lib/config.ts"
import { findBinary } from "../lib/detect.ts"
import { log } from "../lib/logger.ts"
import { pipeWithPrefix } from "./dev.ts"

export interface StartOptions {
  cwd: string
}

export async function start(opts: StartOptions): Promise<void> {
  const cfg = resolvePaths(await loadConfig(opts.cwd), opts.cwd)
  const cacheDir = path.join(opts.cwd, ".tynd", "cache")

  log.blank()
  log.info(`Starting in ${log.cyan("start")} mode (${cfg.runtime})`)

  const binPath = findBinary(cfg.runtime, opts.cwd)
  if (!binPath) {
    log.hint(`tynd-${cfg.runtime} binary not found.`, "Install: bun add @tynd/host")
    process.exit(1)
  }

  if (!existsSync(cfg.frontendDir)) {
    log.hint(
      `Frontend output not found: ${cfg.frontendDir}`,
      "Build it first (e.g. `bun run build:ui`) or run `tynd build`.",
    )
    process.exit(1)
  }
  log.step(`Frontend: static → ${log.gray(cfg.frontendDir)}`)

  let bundlePath: string | null = null
  if (cfg.runtime === "lite") {
    const prodBundle = path.join(cacheDir, "bundle.js")
    const devBundle = path.join(cacheDir, "bundle.dev.js")
    bundlePath = existsSync(prodBundle) ? prodBundle : existsSync(devBundle) ? devBundle : null
    if (!bundlePath) {
      log.hint(
        "No backend bundle found in .tynd/cache/",
        "Run `tynd build` or `tynd dev` once to produce one.",
      )
      process.exit(1)
    }
    log.step(`Backend: ${log.gray(path.relative(opts.cwd, bundlePath))}`)
  } else {
    log.step(`Backend: ${log.gray(path.relative(opts.cwd, cfg.backend))}`)
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
