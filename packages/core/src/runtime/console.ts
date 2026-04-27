/**
 * Redirect every console.* method to stderr so that stdout stays reserved
 * for IPC JSON. Used by full-mode boot; never called from lite (lite has
 * its own minimal console in `quickjs/globals.js`).
 */

// Bun's other console methods (group/trace/table/…) already go to stderr by default.
const METHODS = ["log", "info", "debug", "warn", "error"] as const

function formatArg(a: unknown): string {
  if (a === null) return "null"
  if (a === undefined) return "undefined"
  if (typeof a === "object" || typeof a === "function") {
    try {
      return JSON.stringify(a, null, 0)
    } catch {
      return String(a)
    }
  }
  return String(a)
}

export function redirectConsoleToStderr(): void {
  const con = console as unknown as Record<(typeof METHODS)[number], (...args: unknown[]) => void>
  for (const method of METHODS) {
    con[method] = (...args: unknown[]) => {
      process.stderr.write(`[${method.toUpperCase()}] ${args.map(formatArg).join(" ")}\n`)
    }
  }
}
