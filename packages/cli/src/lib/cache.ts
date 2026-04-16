import { createHash } from "node:crypto"
import { existsSync, mkdirSync, readdirSync, readFileSync, statSync, writeFileSync } from "node:fs"
import path from "node:path"
import * as v from "valibot"
import { log } from "./logger.ts"

const CacheEntrySchema = v.object({
  hash: v.pipe(v.string(), v.minLength(1)),
  updatedAt: v.pipe(v.number(), v.integer(), v.minValue(0)),
})
export type CacheEntry = v.InferOutput<typeof CacheEntrySchema>

/** Directory names always excluded when walking source trees. */
const ALWAYS_EXCLUDE = new Set([
  "node_modules",
  ".tynd",
  ".git",
  "release",
  ".DS_Store",
  "Thumbs.db",
])

/**
 * Compute a deterministic SHA-256 over:
 *  - all files found by recursively walking `dirs` (sorted for determinism)
 *  - individual `files` (each included only if it exists)
 *
 * `excludeDirNames` adds extra directory names to skip during the walk
 * (e.g. the frontend output dir so a cached dist/ doesn't poison the hash).
 */
export function hashSources(
  dirs: string[],
  files: string[],
  excludeDirNames: Set<string> = new Set(),
): string {
  const h = createHash("sha256")
  const exclude = new Set([...ALWAYS_EXCLUDE, ...excludeDirNames])

  for (const dir of dirs) {
    if (!existsSync(dir)) continue
    for (const file of walkSorted(dir, exclude)) {
      h.update(file) // path contributes — catches renames
      h.update(readFileSync(file)) // content
    }
  }

  for (const file of files) {
    if (!existsSync(file)) continue
    h.update(file)
    h.update(readFileSync(file))
  }

  return h.digest("hex")
}

function walkSorted(dir: string, exclude: Set<string>): string[] {
  const out: string[] = []
  let entries: string[]
  try {
    entries = readdirSync(dir).sort()
  } catch {
    return out
  }

  for (const entry of entries) {
    if (exclude.has(entry)) continue
    const abs = path.join(dir, entry)
    try {
      if (statSync(abs).isDirectory()) {
        out.push(...walkSorted(abs, exclude))
      } else {
        out.push(abs)
      }
    } catch {
      /* skip unreadable */
    }
  }
  return out
}

export function readCache(cacheDir: string, key: string): CacheEntry | null {
  const file = path.join(cacheDir, `${key}.json`)
  if (!existsSync(file)) {
    log.debug(`cache miss (${key}): ${file} does not exist`)
    return null
  }
  try {
    const parsed = v.safeParse(CacheEntrySchema, JSON.parse(readFileSync(file, "utf8")))
    if (!parsed.success) {
      log.debug(`cache miss (${key}): schema mismatch in ${file}`)
      return null
    }
    log.debug(`cache hit (${key}): hash=${parsed.output.hash.slice(0, 12)}…`)
    return parsed.output
  } catch {
    log.debug(`cache miss (${key}): failed to parse ${file}`)
    return null
  }
}

export function writeCache(cacheDir: string, key: string, entry: CacheEntry): void {
  try {
    mkdirSync(cacheDir, { recursive: true })
    writeFileSync(path.join(cacheDir, `${key}.json`), JSON.stringify(entry))
    log.debug(`cache write (${key}): hash=${entry.hash.slice(0, 12)}…`)
  } catch {
    /* non-fatal — next build will recompute and miss cache */
  }
}
