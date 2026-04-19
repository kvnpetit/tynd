import { existsSync } from "node:fs"
import path from "node:path"
import type { BundleConfig } from "../lib/config.ts"
import { resolveEnvRef } from "../lib/env-ref.ts"
import { exec } from "../lib/exec.ts"
import { log } from "../lib/logger.ts"
import type { BundleContext } from "./types.ts"

const DEFAULT_TIMESTAMP_URL = "http://timestamp.digicert.com"

/**
 * Sign a Windows `.exe` / `.msi` / NSIS setup with `signtool.exe`. No-op if
 * the project has no `bundle.sign.windows` block — unsigned builds are fine
 * for CI and dev.
 */
export async function signWindows(
  bundleCfg: BundleConfig | undefined,
  file: string,
): Promise<void> {
  const sign = bundleCfg?.sign?.windows
  if (!sign) return
  if (!existsSync(file)) throw new Error(`sign: file not found: ${file}`)

  const signtool = await findSigntool()
  if (!signtool) {
    throw new Error(
      "sign(windows): signtool.exe not found — install Windows SDK or add signtool to PATH",
    )
  }

  const password = resolveEnvRef(sign.password, "sign.windows.password")
  const timestampUrl = sign.timestampUrl ?? DEFAULT_TIMESTAMP_URL

  const args = ["sign", "/fd", "sha256", "/td", "sha256", "/tr", timestampUrl]
  if (sign.certificate.startsWith("cert:")) {
    // `cert:subject=Publisher Name` → subject-name lookup in the cert store.
    const query = sign.certificate.slice(5)
    const [key, value] = query.split("=", 2)
    if (key === "subject" && value) args.push("/n", value)
    else if (key === "sha1" && value) args.push("/sha1", value)
    else throw new Error(`sign(windows): unsupported certificate selector '${query}'`)
  } else {
    const pfx = path.resolve(sign.certificate)
    if (!existsSync(pfx)) throw new Error(`sign(windows): certificate not found: ${pfx}`)
    args.push("/f", pfx)
    if (password) args.push("/p", password)
  }
  args.push(file)

  log.step(`Signing ${log.gray(path.basename(file))}…`)
  await exec(signtool, args, { silent: true })
  log.success(`Signed  -> ${log.cyan(path.basename(file))}`)
}

/**
 * Locate `signtool.exe`. Priority:
 * 1. `SIGNTOOL` env var (user override).
 * 2. Windows SDK default paths — latest version wins.
 * 3. `signtool` resolved via PATH (relies on OS lookup).
 */
async function findSigntool(): Promise<string | null> {
  if (process.platform !== "win32") {
    throw new Error("sign(windows): can only run on a Windows host")
  }

  const envPath = process.env["SIGNTOOL"]
  if (envPath && existsSync(envPath)) return envPath

  const sdkRoots = [
    "C:/Program Files (x86)/Windows Kits/10/bin",
    "C:/Program Files/Windows Kits/10/bin",
  ]
  const arch = process.arch === "arm64" ? "arm64" : "x64"
  for (const root of sdkRoots) {
    if (!existsSync(root)) continue
    const { readdirSync } = await import("node:fs")
    // Iterate in reverse (newest version first — dir names sort lexicographically).
    for (const entry of readdirSync(root).sort().reverse()) {
      const candidate = path.join(root, entry, arch, "signtool.exe")
      if (existsSync(candidate)) return candidate
    }
  }

  // Fallback to PATH lookup; exec() will surface a readable error if missing.
  return "signtool.exe"
}

/** True if this build has any Windows signing configured (for logging). */
export function hasWindowsSigning(ctx: BundleContext): boolean {
  return !!ctx.bundleConfig.sign?.windows
}

/**
 * Sign a macOS binary or `.app` bundle with `codesign`. Optional notarize
 * step submits the signed artifact to Apple's notary service, waits for
 * acceptance, and staples the ticket so offline launches work.
 */
export async function signMacos(
  bundleCfg: BundleConfig | undefined,
  target: string,
): Promise<void> {
  const sign = bundleCfg?.sign?.macos
  if (!sign) return
  if (process.platform !== "darwin") {
    throw new Error("sign(macos): can only run on a macOS host")
  }
  if (!existsSync(target)) throw new Error(`sign: target not found: ${target}`)

  const args = ["--force", "--deep", "--sign", sign.identity, "--options", "runtime", "--timestamp"]
  if (sign.entitlements) {
    const ent = path.resolve(sign.entitlements)
    if (!existsSync(ent)) throw new Error(`sign(macos): entitlements not found: ${ent}`)
    args.push("--entitlements", ent)
  }
  args.push(target)

  log.step(`codesign ${log.gray(path.basename(target))}…`)
  await exec("codesign", args, { silent: true })
  log.success(`Signed  -> ${log.cyan(path.basename(target))}`)

  if (sign.notarize) {
    await notarizeMacos(target, sign.notarize)
  }
}

async function notarizeMacos(
  target: string,
  opts: { appleId: string; password: string; teamId: string },
): Promise<void> {
  const appleId = resolveEnvRef(opts.appleId, "sign.macos.notarize.appleId")
  const password = resolveEnvRef(opts.password, "sign.macos.notarize.password")
  const teamId = resolveEnvRef(opts.teamId, "sign.macos.notarize.teamId")
  if (!appleId || !password || !teamId) {
    throw new Error("sign(macos).notarize: appleId / password / teamId must resolve to non-empty")
  }

  // notarytool requires a .zip or .dmg — wrap .app bundles accordingly.
  const zipPath = `${target}.notarize.zip`
  log.step(`Zipping ${log.gray(path.basename(target))} for notarization…`)
  await exec("ditto", ["-c", "-k", "--keepParent", target, zipPath], { silent: true })

  log.step("Submitting to Apple notary service (can take several minutes)…")
  await exec(
    "xcrun",
    [
      "notarytool",
      "submit",
      zipPath,
      "--apple-id",
      appleId,
      "--password",
      password,
      "--team-id",
      teamId,
      "--wait",
    ],
    { silent: true },
  )

  // `stapler` attaches the notarization ticket so Gatekeeper trusts the app
  // offline — otherwise it has to phone home on every first launch.
  log.step("Stapling notarization ticket…")
  await exec("xcrun", ["stapler", "staple", target], { silent: true })

  // Best-effort cleanup.
  try {
    const { rmSync } = await import("node:fs")
    rmSync(zipPath, { force: true })
  } catch {
    /* intentional: tmp zip, not worth failing the build over */
  }

  log.success(`Notarized -> ${log.cyan(path.basename(target))}`)
}

export function hasMacosSigning(ctx: BundleContext): boolean {
  return !!ctx.bundleConfig.sign?.macos
}
