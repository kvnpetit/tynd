/**
 * Backend call context (contract A4).
 * The native host appends `ctx: { window, webview, origin }` to every call
 * envelope it forwards to the backend. The runtime stores it in an
 * `AsyncLocalStorage` scope for the whole life of the call (handler body,
 * middleware, plugin hooks and stream iteration) so handlers can read the
 * caller identity with `getCallContext()` without threading it through
 * their arguments.
 */

import { AsyncLocalStorage } from "node:async_hooks";
import * as v from "valibot";

/** Identity of the webview that issued the call currently being served. */
export interface CallContext {
  /** Label of the window that owns the calling webview. */
  window: string;
  /** Label of the calling webview (child webviews differ from their window). */
  webview: string;
  /** Origin of the calling page, or `null` when the host could not determine it. */
  origin: string | null;
}

const CallContextSchema = v.object({
  window: v.string(),
  webview: v.string(),
  origin: v.nullable(v.string()),
});

const storage = new AsyncLocalStorage<CallContext>();

/**
 * Validate the untrusted `ctx` field of a call envelope. Anything that is not
 * exactly the documented shape yields `undefined`: an older host simply does
 * not send it, and a malformed value must never reach handlers.
 */
export function parseCallContext(raw: unknown): CallContext | undefined {
  if (raw === undefined || raw === null) return;
  const parsed = v.safeParse(CallContextSchema, raw);
  return parsed.success ? parsed.output : undefined;
}

/** Run `fn` inside a call-context scope. `undefined` leaves the scope empty. */
export function runWithCallContext<T>(ctx: CallContext | undefined, fn: () => T): T {
  // `run(undefined, …)` is not allowed to inherit an outer scope: a call that
  // arrives without `ctx` must observe `undefined`, not a previous caller.
  return storage.exit(() => (ctx ? storage.run(ctx, fn) : fn()));
}

/**
 * Identity of the webview that issued the backend call being served, or
 * `undefined` outside a call (startup code, timers started at module load)
 * and when the host did not provide a context.
 * @example
 * export function whoAmI() {
 *   return getCallContext()?.window ?? "unknown"
 * }
 */
export function getCallContext(): CallContext | undefined {
  return storage.getStore();
}
