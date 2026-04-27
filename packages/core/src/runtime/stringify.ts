/**
 * IPC-safe JSON stringify shared between `runtime/full.ts` and
 * `runtime/lite.ts`. The wire protocol is JSON, so the value must be
 * something `JSON.stringify` accepts.
 *
 * Replacer rules:
 * - `bigint` → string (lossless wire form; callers decide how to re-hydrate)
 *
 * Returns `string` on success, or `null` when the value can't be
 * serialized (circular refs, non-JSON types, stringify throws). Callers
 * must surface the `null` as an explicit error — never forward it as
 * silent `"null"` text.
 */
export function safeStringify(value: unknown): string | null {
  try {
    return JSON.stringify(value, (_key, v) => {
      if (typeof v === "bigint") return v.toString()
      return v
    })
  } catch {
    return null
  }
}
