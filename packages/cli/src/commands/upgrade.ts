import path from "node:path"
import * as v from "valibot"
import { exec } from "../lib/exec.ts"
import { log } from "../lib/logger.ts"
import { confirm } from "../lib/prompt.ts"
import { VERSION } from "../lib/version.ts"

const VersionedSchema = v.object({ version: v.string() })

export interface UpgradeOptions {
  yes: boolean
}

async function fetchLatestVersion(pkg: string): Promise<string | null> {
  try {
    const res = await fetch(`https://registry.npmjs.org/${pkg}/latest`, {
      signal: AbortSignal.timeout(5000),
    })
    if (!res.ok) {
      log.debug(`fetchLatestVersion(${pkg}): HTTP ${res.status}`)
      return null
    }
    const parsed = v.safeParse(VersionedSchema, await res.json())
    if (!parsed.success) {
      log.debug(`fetchLatestVersion(${pkg}): unexpected response shape`)
      return null
    }
    log.debug(`fetchLatestVersion(${pkg}): ${parsed.output.version}`)
    return parsed.output.version
  } catch (e) {
    log.debug(`fetchLatestVersion(${pkg}): ${e}`)
    return null
  }
}

/** Read installed version from the user's node_modules. Returns null if not installed. */
async function readInstalledVersion(pkg: string): Promise<string | null> {
  try {
    const pkgJson = path.join(process.cwd(), "node_modules", pkg, "package.json")
    const parsed = v.safeParse(VersionedSchema, await Bun.file(pkgJson).json())
    return parsed.success ? parsed.output.version : null
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
    readInstalledVersion("@tynd/core"),
    fetchLatestVersion("@tynd/cli"),
    fetchLatestVersion("@tynd/core"),
  ])

  if (!cliLatest || !coreLatest) {
    log.error("Could not reach npm registry.")
    process.exit(1)
  }

  const coreCurrentStr = coreCurrent ?? VERSION

  log.step(
    `@tynd/cli   ${log.gray(cliCurrent)}      → ${Bun.semver.order(cliLatest, cliCurrent) > 0 ? log.cyan(cliLatest) : log.gray(cliLatest)}`,
  )
  log.step(
    `@tynd/core  ${log.gray(coreCurrentStr)} → ${Bun.semver.order(coreLatest, coreCurrentStr) > 0 ? log.cyan(coreLatest) : log.gray(coreLatest)}`,
  )

  // @tynd/host ships the pre-built native binaries for both runtimes.
  const runtimeResults: Array<{ name: string; current: string; latest: string }> = []
  const hostCurrent = await readInstalledVersion("@tynd/host")
  if (hostCurrent !== null) {
    const hostLatest = await fetchLatestVersion("@tynd/host")
    if (hostLatest) {
      log.step(
        `@tynd/host  ${log.gray(hostCurrent)} → ${Bun.semver.order(hostLatest, hostCurrent) > 0 ? log.cyan(hostLatest) : log.gray(hostLatest)}`,
      )
      if (Bun.semver.order(hostLatest, hostCurrent) > 0) {
        runtimeResults.push({ name: "@tynd/host", current: hostCurrent, latest: hostLatest })
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
      corePkgsNeedUpgrade ? `@tynd/cli@${cliLatest}, @tynd/core@${coreLatest}` : null,
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
    installPkgs.push(`@tynd/cli@${cliLatest}`, `@tynd/core@${coreLatest}`)
  }
  for (const r of runtimeResults) installPkgs.push(`${r.name}@${r.latest}`)

  await exec("bun", ["add", ...installPkgs], { cwd: process.cwd() })

  log.blank()
  log.success(`Upgraded: ${installPkgs.join(", ")}`)
  log.blank()
}
