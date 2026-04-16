import { existsSync } from "node:fs"
import path from "node:path"
import * as v from "valibot"

export type Runtime = "full" | "lite"

// Window/menu/tray config lives in the backend (app.start({ window: {...} }))
// because each spawned window can define its own settings programmatically.
// vorn.config.ts only covers build/CLI concerns.
const ConfigSchema = v.object({
  runtime: v.union([v.literal("full"), v.literal("lite")]),
  backend: v.pipe(v.string(), v.minLength(1)),
  frontendDir: v.pipe(v.string(), v.minLength(1)),
  frontendEntry: v.optional(v.pipe(v.string(), v.minLength(1))),
  devUrl: v.optional(v.pipe(v.string(), v.url())),
  devCommand: v.optional(v.pipe(v.string(), v.minLength(1))),
  icon: v.optional(v.pipe(v.string(), v.minLength(1))),
  binaryArgs: v.optional(v.array(v.string())),
})

/**
 * Vorn configuration — runtime-validated via valibot.
 *
 * Prefer editing JSDoc on the schema fields above rather than duplicating
 * them here. Types are inferred from the schema so they never drift.
 */
export type VornConfig = v.InferOutput<typeof ConfigSchema>

const DEFAULTS: VornConfig = {
  runtime: "full",
  backend: "backend/main.ts",
  frontendDir: "frontend",
}

export class ConfigError extends Error {
  readonly issues: readonly string[]
  constructor(issues: string[]) {
    super(`Invalid vorn.config: ${issues.length} issue${issues.length > 1 ? "s" : ""}`)
    this.name = "ConfigError"
    this.issues = issues
  }
}

/** Load vorn.config.ts from the project directory (defaults to cwd). */
export async function loadConfig(cwd = process.cwd()): Promise<VornConfig> {
  const candidates = [path.resolve(cwd, "vorn.config.ts"), path.resolve(cwd, "vorn.config.js")]

  for (const file of candidates) {
    if (existsSync(file)) {
      const mod = (await import(file)) as { default?: unknown }
      const merged = { ...DEFAULTS, ...((mod.default ?? {}) as Partial<VornConfig>) }
      return validateConfig(merged)
    }
  }

  return { ...DEFAULTS }
}

/** Resolve config paths relative to cwd. */
export function resolvePaths(cfg: VornConfig, cwd: string): VornConfig {
  return {
    ...cfg,
    backend: path.resolve(cwd, cfg.backend),
    frontendDir: path.resolve(cwd, cfg.frontendDir),
  }
}

/**
 * Runtime-validate a freshly-loaded config. Throws ConfigError listing every
 * bad field so users fix them all in one edit cycle.
 */
export function validateConfig(raw: unknown): VornConfig {
  const result = v.safeParse(ConfigSchema, raw)
  if (result.success) return result.output

  const issues = result.issues.map((i) => {
    const path = i.path?.map((p) => p.key).join(".") ?? ""
    return path ? `${path}: ${i.message}` : i.message
  })
  throw new ConfigError(issues)
}
