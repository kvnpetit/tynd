/**
 * Tynd internal logger — dev-only, zero cost in production.
 *
 * Detection (first match wins):
 *   1. Browser / Vite  — import.meta.env.DEV (true in dev server, false in prod build)
 *   2. Bun / full mode — TYND_DEV_URL env var set by `tynd dev`
 *   3. QuickJS / lite  — __TYND_DEV__ global injected as a Bun.build define by `tynd dev`
 *
 * In production none of these are set, so all tynd.warn/error calls become no-ops
 * and the framework name never leaks to end users.
 */

const _g = globalThis as Record<string, unknown>

export const IS_DEV: boolean = (() => {
  // 1. Browser (Vite) — replaced at build time: true in dev server, false in prod build
  try {
    if ((import.meta as { env?: { DEV?: boolean } }).env?.DEV === true) return true
  } catch {
    /* QuickJS: import.meta is replaced with {} — no-op */
  }
  // 2. Bun subprocess (full mode) — CLI sets TYND_DEV_URL during `tynd dev`
  if (typeof process !== "undefined" && !!process.env?.["TYND_DEV_URL"]) return true
  // 3. QuickJS (lite mode) — CLI injects `globalThis.__TYND_DEV__ = true` at bundle time
  if (_g["__TYND_DEV__"] === true) return true
  return false
})()

export const tynd = {
  /** Log a framework warning. Visible in DevTools / stderr only in dev mode. */
  warn(msg: string): void {
    if (IS_DEV) console.warn(`[tynd] ${msg}`)
  },
  /** Log a framework error. Visible in DevTools / stderr only in dev mode. */
  error(msg: string): void {
    if (IS_DEV) console.error(`[tynd] ${msg}`)
  },
  /** Log a framework info message. Visible in DevTools / stderr only in dev mode. */
  info(msg: string): void {
    if (IS_DEV) console.info(`[tynd] ${msg}`)
  },
}
