/**
 * Resolve a config value that may be a literal string or an `env:NAME`
 * reference. Returns the resolved string or throws if the env var is
 * unset — fail fast at build time instead of shipping an unsigned build.
 */
export function resolveEnvRef(value: string | undefined, label: string): string | undefined {
  if (value === undefined) return undefined
  if (!value.startsWith("env:")) return value
  const name = value.slice(4)
  const resolved = process.env[name]
  if (!resolved) {
    throw new Error(
      `${label}: env var '${name}' is not set (referenced as '${value}' in tynd.config.ts)`,
    )
  }
  return resolved
}
