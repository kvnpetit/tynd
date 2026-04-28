/**
 * Shared lifecycle state: emit function wired by whichever runtime
 * (`full` or `lite`) is active, plus the onReady/onClose handler lists
 * and "has fired" flags. Kept separate so the public API (`app`,
 * `createEmitter` in index.ts) can read/write the state without
 * pulling in runtime-specific code paths.
 */

import mitt from "mitt";
import { clearFrontendListeners } from "./frontend-events";

type EmitFn = (name: string, payload: unknown, to?: string) => void;

let _emitFn: EmitFn | null = null;
export function setEmitFn(fn: EmitFn): void {
  _emitFn = fn;
}
export function getEmitFn(): EmitFn | null {
  return _emitFn;
}

// Mitt provides the pub/sub primitive; we wrap it in `pushReady`/`pushClose`
// to keep the existing array-style call sites in `index.ts` (`onReadyFns.push`
// / `onCloseFns.push`) working without breaking the public surface. We don't
// use the payload — `undefined` keeps Biome's noConfusingVoidType happy.
type LifecycleEvents = { ready: undefined; close: undefined };
const _bus = mitt<LifecycleEvents>();

/**
 * Adapter that mimics the old `Array.push`-based API. Using the same
 * shape means the `app.onReady`/`app.onClose` call sites in `index.ts`
 * stay untouched.
 */
const arrayLike = (event: keyof LifecycleEvents) => ({
  push(fn: () => void) {
    _bus.on(event, fn);
  },
});

export const onReadyFns = arrayLike("ready");
export const onCloseFns = arrayLike("close");

export const lifecycleFlags = { readyFired: false, closeFired: false };

/** Fire every registered `app.onReady` handler; swallow individual errors. */
export function fireReady(): void {
  lifecycleFlags.readyFired = true;
  // Snapshot + isolate per-handler errors so one bad listener can't
  // block the others. mitt itself doesn't catch inside `emit`.
  const handlers = (_bus.all.get("ready") ?? []).slice();
  _bus.all.delete("ready");
  for (const fn of handlers as Array<() => void>) {
    try {
      fn();
    } catch {
      /* best-effort: one handler's error must not block the others */
    }
  }
}

/** Fire every registered `app.onClose` handler; swallow individual errors. */
export function fireClose(): void {
  lifecycleFlags.closeFired = true;
  const handlers = (_bus.all.get("close") ?? []).slice();
  _bus.all.delete("close");
  for (const fn of handlers as Array<() => void>) {
    try {
      fn();
    } catch {
      /* best-effort: one handler's error must not block the others */
    }
  }
  // Drop any frontend-emit handlers so the listener map doesn't outlive
  // the app. User code that relies on its own unsubscribe pattern is
  // unaffected (that path already ran during normal teardown).
  clearFrontendListeners();
}
