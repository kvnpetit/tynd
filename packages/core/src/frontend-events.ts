/**
 * Frontend -> backend event bus. Backend registers handlers via
 * `onFrontendEmit`; the frontend emits via `createFrontendEmitter`.
 *
 * Wire format is a regular RPC call with a reserved fn name
 * (`__tynd_fe_emit__`) so it travels over the existing IPC path without
 * adding a new message type — runtimes just intercept the reserved name
 * before looking it up in the user module.
 */

import { tynd } from "./logger.js"

export const FRONTEND_EMIT_FN = "__tynd_fe_emit__"

type Listener = (payload: unknown) => void
const _listeners = new Map<string, Set<Listener>>()

export function onFrontendEmit<T>(name: string, handler: (payload: T) => void): () => void {
  let bucket = _listeners.get(name)
  if (!bucket) {
    bucket = new Set()
    _listeners.set(name, bucket)
  }
  bucket.add(handler as Listener)
  return () => bucket!.delete(handler as Listener)
}

/** Called by the runtime when the frontend posts a reserved emit call. */
export function dispatchFrontendEmit(name: string, payload: unknown): void {
  const bucket = _listeners.get(name)
  if (!bucket) return
  for (const h of bucket) {
    try {
      h(payload)
    } catch (e) {
      tynd.error(`onFrontendEmit(${name}): ${String(e)}`)
    }
  }
}

// Lite runtime has no module-import bridge back to the backend bundle — it
// uses a global hook set by globals.js. Registering the dispatcher on the
// global so both runtimes land in the same bucket.
const globalAny = globalThis as Record<string, unknown>
globalAny["__tynd_fe_dispatch__"] = dispatchFrontendEmit
