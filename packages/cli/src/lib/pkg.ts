import { existsSync } from "node:fs"
import path from "node:path"
import * as v from "valibot"
import { log } from "./logger.ts"

const StringRecord = v.record(v.string(), v.string())

export const PackageJsonSchema = v.looseObject({
  name: v.optional(v.string()),
  version: v.optional(v.string()),
  scripts: v.optional(StringRecord),
  dependencies: v.optional(StringRecord),
  devDependencies: v.optional(StringRecord),
})
export type PackageJson = v.InferOutput<typeof PackageJsonSchema>

/** Load and validate a `package.json`. Returns null if missing or malformed. */
export async function loadPackageJson(cwd: string): Promise<PackageJson | null> {
  const p = path.join(cwd, "package.json")
  if (!existsSync(p)) {
    log.debug(`loadPackageJson: ${p} not found`)
    return null
  }
  try {
    const raw = await Bun.file(p).json()
    const parsed = v.safeParse(PackageJsonSchema, raw)
    if (!parsed.success) {
      log.debug(`loadPackageJson: schema mismatch in ${p}`)
      return null
    }
    return parsed.output
  } catch (e) {
    log.debug(`loadPackageJson: failed to read ${p}: ${e}`)
    return null
  }
}

/** Combined dependencies + devDependencies. */
export function allDeps(pkg: PackageJson): Record<string, string> {
  return { ...pkg.dependencies, ...pkg.devDependencies }
}
