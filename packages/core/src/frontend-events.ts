/**
 * Frontend -> backend event bus. Backend registers handlers via
 * `onFrontendEmit`; the frontend emits via `createFrontendEmitter`.
 * Wire format is a regular RPC call with a reserved fn name
 * (`__tynd_fe_emit__`) so it travels over the existing IPC path without
 * adding a new message type — runtimes just intercept the reserved name
 * before looking it up in the user module.
 * The bus is a tiny local Map<Set> so the core package does not need an
 * external event-emitter dependency for this narrow internal use.
 */

import { tynd } from "./logger";
import { sanitizeError } from "./runtime/error";

export const FRONTEND_EMIT_FN = "__tynd_fe_emit__";

type FrontendHandler = (payload: unknown) => void;
const _bus = new Map<string, Set<FrontendHandler>>();

/**
 * Register a backend handler for a reserved frontend event. Returns an
 * unsubscribe function — call it when the handler is no longer needed
 * (e.g. during a reload sequence) to keep the listener map bounded.
 * Handlers survive frontend navigations by design: the backend owns the
 * listener lifetime, not the page. If you need to reset the map (tests,
 * teardown), use `clearFrontendListeners`.
 */
export function onFrontendEmit<T>(name: string, handler: (payload: T) => void): () => void {
  const wrapped = handler as FrontendHandler;
  let bucket = _bus.get(name);
  if (!bucket) {
    bucket = new Set();
    _bus.set(name, bucket);
  }
  bucket.add(wrapped);
  return () => {
    const current = _bus.get(name);
    current?.delete(wrapped);
    if (current?.size === 0) _bus.delete(name);
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
    _bus.clear();
    return;
  }
  _bus.delete(name);
}

/** Called by the runtime when the frontend posts a reserved emit call. */
export function dispatchFrontendEmit(name: string, payload: unknown): void {
  const bucket = _bus.get(name);
  if (!bucket || bucket.size === 0) return;
  // Snapshot so a handler that unsubscribes itself does not skip its neighbour.
  for (const h of Array.from(bucket)) {
    try {
      h(payload);
    } catch (e) {
      // Errors thrown from frontend-emit handlers stay LOCAL — they're
      // logged to stderr via the tynd logger and not propagated back to
      // the frontend, since the dispatcher has no IPC return channel.
      // We sanitize on the way out anyway so any future change that DOES
      // surface this to the host (or a network-attached log sink) can't
      // accidentally start leaking stack frames / filesystem paths.
      const { name: errName, message } = sanitizeError(e);
      tynd.error(`onFrontendEmit(${name}): ${errName}: ${message}`);
    }
  }
}
