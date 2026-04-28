/**
 * Frontend -> backend event bus. Backend registers handlers via
 * `onFrontendEmit`; the frontend emits via `createFrontendEmitter`.
 *
 * Wire format is a regular RPC call with a reserved fn name
 * (`__tynd_fe_emit__`) so it travels over the existing IPC path without
 * adding a new message type — runtimes just intercept the reserved name
 * before looking it up in the user module.
 */

import { tynd } from "./logger";

export const FRONTEND_EMIT_FN = "__tynd_fe_emit__";

type Listener = (payload: unknown) => void;
const _listeners = new Map<string, Set<Listener>>();

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
  let bucket = _listeners.get(name);
  if (!bucket) {
    bucket = new Set();
    _listeners.set(name, bucket);
  }
  bucket.add(handler as Listener);
  return () => {
    bucket?.delete(handler as Listener);
    if (bucket?.size === 0) _listeners.delete(name);
  };
}

/**
 * Drop every registered frontend-emit handler, or only those for `name`
 * when specified. Intended for test teardown and the `app.onClose`
 * lifecycle — regular code should prefer the unsubscribe callback
 * returned by `onFrontendEmit`.
 */
export function clearFrontendListeners(name?: string): void {
  if (name === undefined) {
    _listeners.clear();
    return;
  }
  _listeners.delete(name);
}

/** Called by the runtime when the frontend posts a reserved emit call. */
export function dispatchFrontendEmit(name: string, payload: unknown): void {
  const bucket = _listeners.get(name);
  if (!bucket) return;
  for (const h of bucket) {
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
