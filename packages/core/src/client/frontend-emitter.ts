/**
 * Frontend -> backend event bus. Mirrors the backend's `createEmitter` so
 * code on either side feels the same. Internally posts a reserved RPC call
 * (`__tynd_fe_emit__`) that the runtime intercepts before user-module
 * lookup.
 */

import type { EmitterMap } from "../types.js"

const FRONTEND_EMIT_FN = "__tynd_fe_emit__"

export interface FrontendEmitter<T extends EmitterMap> {
  emit<K extends keyof T>(event: K & string, payload: T[K]): void
}

/**
 * @example
 *   // Backend (import from "@tynd/core"):
 *   onFrontendEmit<{ toast: string }>("toast", (text) => logger.info(text))
 *
 *   // Frontend (import from "@tynd/core/client"):
 *   const frontend = createFrontendEmitter<{ toast: string }>()
 *   frontend.emit("toast", "Saved!")
 */
export function createFrontendEmitter<T extends EmitterMap>(): FrontendEmitter<T> {
  return {
    emit(event, payload) {
      // Fire-and-forget — we don't care about the return value, but awaiting
      // the promise catches IPC transport errors for dev-time debugging.
      void window.__tynd__.call(FRONTEND_EMIT_FN, [event, payload])
    },
  }
}
