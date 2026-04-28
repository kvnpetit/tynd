/**
 * Frontend -> backend event bus. Backend registers handlers via
 * `onFrontendEmit`; the frontend emits via `createFrontendEmitter`.
 *
 * Wire format is a regular RPC call with a reserved fn name
 * (`__tynd_fe_emit__`) so it travels over the existing IPC path without
 * adding a new message type — runtimes just intercept the reserved name
 * before looking it up in the user module.
 *
 * The bus itself is a tiny `mitt` emitter; we wrap it only to keep the
 * unsubscribe-callback ergonomics of the previous custom Map<Set>
 * implementation and to fan out errors via the tynd logger.
 */

import mitt, { type Handler } from "mitt";
import { tynd } from "./logger";

export const FRONTEND_EMIT_FN = "__tynd_fe_emit__";

const _bus = mitt<Record<string, unknown>>();

/**
 * Register a backend handler for a reserved frontend event. Returns an
 * unsubscribe function — call it when the handler is no longer needed
 * (e.g. during a reload sequence) to keep the listener map bounded.
 *
 * Handlers survive frontend navigations by design: the backend owns the
 * listener lifetime, not the page. If you need to reset the map (tests,
 * teardown), use `clearFrontendListeners`.
 */
export function onFrontendEmit<T>(name: string, handler: (payload: T) => void): () => void {
  const wrapped = handler as Handler<unknown>;
  _bus.on(name, wrapped);
  return () => _bus.off(name, wrapped);
}

/**
 * Drop every registered frontend-emit handler, or only those for `name`
 * when specified. Intended for test teardown and the `app.onClose`
 * lifecycle — regular code should prefer the unsubscribe callback
 * returned by `onFrontendEmit`.
 */
export function clearFrontendListeners(name?: string): void {
  if (name === undefined) {
    _bus.all.clear();
    return;
  }
  _bus.all.delete(name);
}

/** Called by the runtime when the frontend posts a reserved emit call. */
export function dispatchFrontendEmit(name: string, payload: unknown): void {
  const bucket = _bus.all.get(name);
  if (!bucket || bucket.length === 0) return;
  // Snapshot the handler list before iterating so a handler that
  // unsubscribes itself doesn't skip its neighbour. mitt mutates the
  // array in-place on `off`, so mirroring the previous Set-based
  // semantics requires a copy.
  for (const h of bucket.slice() as Array<Handler<unknown>>) {
    try {
      h(payload);
    } catch (e) {
      // Prefer the stack (when we have one) so the source-map-enhanced
      // Bun stderr points right at the user's handler. Falls back to the
      // message / String cast for non-Error throws.
      const detail = e instanceof Error ? (e.stack ?? e.message) : String(e);
      tynd.error(`onFrontendEmit(${name}): ${detail}`);
    }
  }
}

// Lite runtime has no module-import bridge back to the backend bundle — it
// uses a global hook set by globals.js. Registering the dispatcher on the
// global so both runtimes land in the same bucket.
const globalAny = globalThis as Record<string, unknown>;
globalAny.__tynd_fe_dispatch__ = dispatchFrontendEmit;
