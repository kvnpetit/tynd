/**
 * Resolve a config value that may be a literal string or an `env:NAME`
 * reference. Returns the resolved string or throws if the env var is
 * missing OR empty — both cases fail fast so builds don't ship a binary
 * signed with a blank password or submitted to notary without credentials.
 */
export function resolveEnvRef(value: string | undefined, label: string): string | undefined {
  if (value === undefined) return undefined
  if (!value.startsWith("env:")) return value
  const name = value.slice(4)
  const resolved = process.env[name]
  if (resolved === undefined) {
    throw new Error(
      `${label}: env var '${name}' is not set (referenced as '${value}' in tynd.config.ts)`,
    )
  }
  if (resolved === "") {
    throw new Error(
      `${label}: env var '${name}' is empty (referenced as '${value}' in tynd.config.ts — ` +
        `unset the variable entirely if you meant "no value", or provide a real one)`,
    )
  }
  return resolved
}
