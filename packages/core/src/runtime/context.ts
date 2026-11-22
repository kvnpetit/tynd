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
  /** Label of the window that owns the calling webview. Drive it with `getWindow(ctx.window)`. */
  window: string;
  /** Label of the calling webview (child webviews differ from their window). */
  webview: string;
  /**
   * Origin of the calling frame when it shows remote content
   * (`"https://example.com"`), `null` for the app's own pages.
   */
  origin: string | null;
  /** `true` when the call comes from the app's own pages (bundled or dev server), `false` for remote content. */
  isAppContent: boolean;
  /**
   * Aborted when the caller gives up: `handle.cancel()`, an `AbortSignal` or
   * `timeout` passed through `api.withOptions(...)`, or the calling window
   * closing or navigating away. Pass it to `fetch`, `Bun.spawn`, timers…
   * The client has already rejected when it fires: the handler's result is
   * dropped.
   */
  signal: AbortSignal;
}

const CallIdentitySchema = v.object({
  window: v.string(),
  webview: v.string(),
  origin: v.nullable(v.string()),
});

const storage = new AsyncLocalStorage<CallContext>();

/**
 * Validate the untrusted `ctx` field of a call envelope. Anything that is not
 * exactly the documented shape yields `undefined`: an older host simply does
 * not send it, and a malformed value must never reach handlers. `signal` is
 * the call's own abort signal (owned by the runtime, never sent by the host).
 */
export function parseCallContext(raw: unknown, signal: AbortSignal): CallContext | undefined {
  if (raw === undefined || raw === null) return;
  const parsed = v.safeParse(CallIdentitySchema, raw);
  if (!parsed.success) return;
  return { ...parsed.output, isAppContent: parsed.output.origin === null, signal };
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
 * export async function search(query: string) {
 *   const ctx = getCallContext()
 *   const res = await fetch(`https://api.example.com/?q=${query}`, { signal: ctx?.signal })
 *   return res.json()
 * }
 */
export function getCallContext(): CallContext | undefined {
  return storage.getStore();
}
