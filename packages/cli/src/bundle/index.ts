import { mkdirSync } from "node:fs"
import type { Platform } from "../lib/detect.ts"
import { log } from "../lib/logger.ts"
import { bundleApp } from "./app.ts"
import { bundleAppImage } from "./appimage.ts"
import { bundleDeb } from "./deb.ts"
import { bundleDmg } from "./dmg.ts"
import { bundleMsi } from "./msi.ts"
import { bundleNsis } from "./nsis.ts"
import { bundleRpm } from "./rpm.ts"
import { ALL_TARGETS, type BundleContext, type BundleTarget, TARGETS_BY_PLATFORM } from "./types.ts"

// Flag semantics: absent = null, bare `--bundle` = "all", otherwise a
// comma list of target names.
export function parseBundleFlag(raw: string | boolean | undefined): "all" | BundleTarget[] | null {
  if (raw === undefined || raw === false) return null
  if (raw === true || raw === "all") return "all"
  const parts = raw
    .split(",")
    .map((s) => s.trim().toLowerCase())
    .filter(Boolean)
  for (const p of parts) {
    if (!ALL_TARGETS.includes(p as BundleTarget)) {
      throw new Error(`unknown bundle target "${p}" — valid: ${ALL_TARGETS.join(", ")} (or "all")`)
    }
  }
  return parts as BundleTarget[]
}

export function resolveTargets(
  requested: "all" | readonly BundleTarget[],
  platform: Platform,
): readonly BundleTarget[] {
  const applicable = TARGETS_BY_PLATFORM[platform]
  if (requested === "all") return applicable
  for (const t of requested) {
    if (!applicable.includes(t)) {
      throw new Error(
        `bundle target "${t}" is not applicable on ${platform} — valid here: ${applicable.join(", ")}`,
      )
    }
  }
  return requested
}

export async function runBundle(
  ctx: BundleContext,
  targets: readonly BundleTarget[],
): Promise<string[]> {
  mkdirSync(ctx.outDir, { recursive: true })
  const outputs: string[] = []

  log.blank()
  log.info(`Bundling: ${log.cyan(targets.join(", "))}`)

  // .dmg wraps the .app, so build the .app once even if only .dmg was asked.
  let appPath: string | null = null
  if (targets.includes("app") || targets.includes("dmg")) {
    appPath = await bundleApp(ctx)
    if (targets.includes("app")) outputs.push(appPath)
  }
  if (targets.includes("dmg")) {
    if (!appPath) throw new Error("dmg requires app bundle (internal error)")
    outputs.push(await bundleDmg(ctx, appPath))
  }

  if (targets.includes("deb")) outputs.push(await bundleDeb(ctx))
  if (targets.includes("rpm")) outputs.push(await bundleRpm(ctx))
  if (targets.includes("appimage")) outputs.push(await bundleAppImage(ctx))

  if (targets.includes("nsis")) outputs.push(await bundleNsis(ctx))
  if (targets.includes("msi")) outputs.push(await bundleMsi(ctx))

  return outputs
}
