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

function compareSemver(a: string, b: string): number {
  const cleanA = a.split("-")[0]!
  const cleanB = b.split("-")[0]!
  const pa = cleanA.split(".").map((n) => Number.parseInt(n, 10) || 0)
  const pb = cleanB.split(".").map((n) => Number.parseInt(n, 10) || 0)
  for (let i = 0; i < 3; i++) {
    const da = pa[i] ?? 0
    const db = pb[i] ?? 0
    if (da > db) return 1
    if (da < db) return -1
  }
  const hasPreA = a.includes("-")
  const hasPreB = b.includes("-")
  if (!hasPreA && hasPreB) return 1
  if (hasPreA && !hasPreB) return -1
  return 0
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
    `@vorn/cli   ${log.gray(cliCurrent)}      → ${compareSemver(cliLatest, cliCurrent) > 0 ? log.cyan(cliLatest) : log.gray(cliLatest)}`,
  )
  log.step(
    `@vorn/core  ${log.gray(coreCurrentStr)} → ${compareSemver(coreLatest, coreCurrentStr) > 0 ? log.cyan(coreLatest) : log.gray(coreLatest)}`,
  )

  // Check optional runtime packages
  const runtimePkgs = ["@vorn/full", "@vorn/lite"] as const
  const runtimeResults: Array<{ name: string; current: string; latest: string }> = []
  for (const pkg of runtimePkgs) {
    const current = await readInstalledVersion(pkg)
    if (current === null) continue
    const latest = await fetchLatestVersion(pkg)
    if (latest) {
      log.step(
        `${pkg}  ${log.gray(current)} → ${compareSemver(latest, current) > 0 ? log.cyan(latest) : log.gray(latest)}`,
      )
      if (compareSemver(latest, current) > 0) runtimeResults.push({ name: pkg, current, latest })
    }
  }

  log.blank()

  const corePkgsNeedUpgrade =
    compareSemver(cliLatest, cliCurrent) > 0 || compareSemver(coreLatest, coreCurrentStr) > 0

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
