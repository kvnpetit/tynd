import { existsSync, rmSync } from "node:fs"
import path from "node:path"
import { exec } from "../lib/exec.ts"
import { log } from "../lib/logger.ts"
import type { BundleContext } from "./types.ts"

export async function bundleDmg(ctx: BundleContext, appPath: string): Promise<string> {
  const dmgPath = path.join(ctx.outDir, `${ctx.displayName}-${ctx.version}.dmg`)
  if (existsSync(dmgPath)) rmSync(dmgPath, { force: true })

  await exec(
    "hdiutil",
    [
      "create",
      "-volname",
      ctx.displayName,
      "-srcfolder",
      appPath,
      "-ov",
      "-format",
      "UDZO",
      dmgPath,
    ],
    { silent: true },
  )

  log.success(`DMG      -> ${log.cyan(`release/${path.basename(dmgPath)}`)}`)
  return dmgPath
}
