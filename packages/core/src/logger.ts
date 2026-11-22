/**
 * Tynd internal logger — dev-only, zero cost in production.
 * Detection (first match wins):
 *   1. Browser / Vite  — import.meta.env.DEV (true in dev server, false in prod build)
 *   2. Bun backend — TYND_DEV_URL env var set by `tynd dev`
 * In production none of these are set, so all tynd.warn/error calls become no-ops
 * and the framework name never leaks to end users.
 */

const _g = globalThis as Record<string, unknown>;

const IS_DEV: boolean = (() => {
  // 1. Browser (Vite) — replaced at build time: true in dev server, false in prod build
  try {
    if ((import.meta as { env?: { DEV?: boolean } }).env?.DEV === true) return true;
  } catch {
    /* Some bundled contexts omit import.meta.env — no-op. */
  }
  // 2. Bun subprocess — CLI sets TYND_DEV_URL during `tynd dev`
  if (typeof process !== "undefined" && process.env?.TYND_DEV_URL) return true;
  // Bundled development contexts may inject `globalThis.__TYND_DEV__`.
  if (_g.__TYND_DEV__ === true) return true;
  return false;
})();

export const tynd = {
  /** Log a framework warning. Visible in DevTools / stderr only in dev mode. */
  warn(_msg: string): void {
    // biome-ignore lint/suspicious/noConsole: framework-level dev logging
    if (IS_DEV) console.warn(`[tynd] ${_msg}`);
  },
  /** Log a framework error. Visible in DevTools / stderr only in dev mode. */
  error(_msg: string): void {
    // biome-ignore lint/suspicious/noConsole: framework-level dev logging
    if (IS_DEV) console.error(`[tynd] ${_msg}`);
  },
  /** Log a framework info message. Visible in DevTools / stderr only in dev mode. */
  info(_msg: string): void {
    // biome-ignore lint/suspicious/noConsole: framework-level dev logging
    if (IS_DEV) console.info(`[tynd] ${_msg}`);
  },
};
