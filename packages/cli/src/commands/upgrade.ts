import path from "node:path"
import { exec } from "../lib/exec.ts"
import { log } from "../lib/logger.ts"
import { confirm } from "../lib/prompt.ts"
import { VERSION } from "../lib/version.ts"

export interface UpgradeOptions {
  yes: boolean
}

async function fetchLatestVersion(pkg: string): Promise<string | null> {
  try {
    const res = await fetch(`https://registry.npmjs.org/${pkg}/latest`, {
      signal: AbortSignal.timeout(5000),
    })
    if (!res.ok) return null
    const data = (await res.json()) as { version: string }
    return data.version ?? null
  } catch {
    return null
  }
}

/** Read installed version from the user's node_modules. Returns null if not installed. */
async function readInstalledVersion(pkg: string): Promise<string | null> {
  try {
    const pkgJson = path.join(process.cwd(), "node_modules", pkg, "package.json")
    const data = (await Bun.file(pkgJson).json()) as { version?: string }
    return data.version ?? null
  } catch {
    return null
  }
}

export async function upgrade(opts: UpgradeOptions): Promise<void> {
  log.blank()
  log.info("Checking for updates…")
  log.blank()

  const cliCurrent = VERSION
  const [coreCurrent, cliLatest, coreLatest] = await Promise.all([
    readInstalledVersion("@vorn/core"),
    fetchLatestVersion("@vorn/cli"),
    fetchLatestVersion("@vorn/core"),
  ])

  if (!cliLatest || !coreLatest) {
    log.error("Could not reach npm registry.")
    process.exit(1)
  }

  const coreCurrentStr = coreCurrent ?? VERSION

  log.step(
    `@vorn/cli   ${log.gray(cliCurrent)}      → ${Bun.semver.order(cliLatest, cliCurrent) > 0 ? log.cyan(cliLatest) : log.gray(cliLatest)}`,
  )
  log.step(
    `@vorn/core  ${log.gray(coreCurrentStr)} → ${Bun.semver.order(coreLatest, coreCurrentStr) > 0 ? log.cyan(coreLatest) : log.gray(coreLatest)}`,
  )

  // @vorn/host ships the pre-built native binaries for both runtimes.
  const runtimeResults: Array<{ name: string; current: string; latest: string }> = []
  const hostCurrent = await readInstalledVersion("@vorn/host")
  if (hostCurrent !== null) {
    const hostLatest = await fetchLatestVersion("@vorn/host")
    if (hostLatest) {
      log.step(
        `@vorn/host  ${log.gray(hostCurrent)} → ${Bun.semver.order(hostLatest, hostCurrent) > 0 ? log.cyan(hostLatest) : log.gray(hostLatest)}`,
      )
      if (Bun.semver.order(hostLatest, hostCurrent) > 0) {
        runtimeResults.push({ name: "@vorn/host", current: hostCurrent, latest: hostLatest })
      }
    }
  }

  log.blank()

  const corePkgsNeedUpgrade =
    Bun.semver.order(cliLatest, cliCurrent) > 0 || Bun.semver.order(coreLatest, coreCurrentStr) > 0

  const needsUpgrade = corePkgsNeedUpgrade || runtimeResults.length > 0

  if (!needsUpgrade) {
    log.success("Already on latest version.")
    log.blank()
    return
  }

  if (!opts.yes) {
    const pkgList = [
      corePkgsNeedUpgrade ? `@vorn/cli@${cliLatest}, @vorn/core@${coreLatest}` : null,
      ...runtimeResults.map((r) => `${r.name}@${r.latest}`),
    ]
      .filter(Boolean)
      .join(", ")
    const ok = await confirm(`Upgrade ${pkgList}?`)
    if (!ok) {
      log.warn("Cancelled.")
      log.blank()
      return
    }
  }

  log.step("Installing…")
  const installPkgs: string[] = []
  if (corePkgsNeedUpgrade) {
    installPkgs.push(`@vorn/cli@${cliLatest}`, `@vorn/core@${coreLatest}`)
  }
  for (const r of runtimeResults) installPkgs.push(`${r.name}@${r.latest}`)

  await exec("bun", ["add", ...installPkgs], { cwd: process.cwd() })

  log.blank()
  log.success(`Upgraded: ${installPkgs.join(", ")}`)
  log.blank()
}
