/**
 * Shared lifecycle state: emit function wired by whichever runtime
 * (`full` or `lite`) is active, plus the onReady/onClose handler lists
 * and "has fired" flags. Kept separate so the public API (`app`,
 * `createEmitter` in index.ts) can read/write the state without
 * pulling in runtime-specific code paths.
 */

type EmitFn = (name: string, payload: unknown) => void

let _emitFn: EmitFn | null = null
export function setEmitFn(fn: EmitFn): void {
  _emitFn = fn
}
export function getEmitFn(): EmitFn | null {
  return _emitFn
}

export const onReadyFns: Array<() => void> = []
export const onCloseFns: Array<() => void> = []

export const lifecycleFlags = { readyFired: false, closeFired: false }

/** Fire every registered `app.onReady` handler; swallow individual errors. */
export function fireReady(): void {
  lifecycleFlags.readyFired = true
  for (const fn of onReadyFns) {
    try {
      fn()
    } catch {
      /* best-effort: one handler's error must not block the others */
    }
  }
}

/** Fire every registered `app.onClose` handler; swallow individual errors. */
export function fireClose(): void {
  lifecycleFlags.closeFired = true
  for (const fn of onCloseFns) {
    try {
      fn()
    } catch {
      /* best-effort: one handler's error must not block the others */
    }
  }
}
