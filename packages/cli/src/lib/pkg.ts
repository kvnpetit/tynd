import { existsSync } from "node:fs"
import path from "node:path"
import * as v from "valibot"
import { log } from "./logger.ts"

const StringRecord = v.record(v.string(), v.string())

const AuthorField = v.union([
  v.string(),
  v.looseObject({
    name: v.optional(v.string()),
    email: v.optional(v.string()),
    url: v.optional(v.string()),
  }),
])

export const PackageJsonSchema = v.looseObject({
  name: v.optional(v.string()),
  version: v.optional(v.string()),
  description: v.optional(v.string()),
  author: v.optional(AuthorField),
  homepage: v.optional(v.string()),
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

export interface NormalizedAuthor {
  name: string
  email?: string
  url?: string
}

/** Parse npm's `author` field: accepts either the object form or the
 *  `"Name <email> (url)"` string form. Returns null if the name is missing. */
export function normalizeAuthor(raw: unknown): NormalizedAuthor | null {
  if (!raw) return null
  if (typeof raw === "string") {
    const m = raw.match(/^\s*([^<(]+?)\s*(?:<([^>]+)>)?\s*(?:\(([^)]+)\))?\s*$/)
    if (!m) return null
    const name = m[1]?.trim() ?? ""
    if (!name) return null
    const author: NormalizedAuthor = { name }
    if (m[2]) author.email = m[2].trim()
    if (m[3]) author.url = m[3].trim()
    return author
  }
  if (typeof raw === "object") {
    const o = raw as Record<string, unknown>
    const name = typeof o["name"] === "string" ? o["name"] : null
    if (!name) return null
    const author: NormalizedAuthor = { name }
    if (typeof o["email"] === "string") author.email = o["email"]
    if (typeof o["url"] === "string") author.url = o["url"]
    return author
  }
  return null
}
