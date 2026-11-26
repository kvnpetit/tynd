/**
 * Shared lifecycle state: emit function wired by Bun runtime, plus
 * the onReady/onClose handler lists
 * and "has fired" flags. Kept separate so the public API (`app`,
 * `createEmitter` in index.ts) can read/write the state without
 * pulling in runtime-specific code paths.
 */

import { clearFrontendListeners } from "./frontend-events";
import { tynd } from "./logger";
import { fireHook } from "./runtime/hooks";

type EmitFn = (name: string, payload: unknown, to?: string) => void;

let _emitFn: EmitFn | null = null;
export function setEmitFn(fn: EmitFn): void {
  _emitFn = fn;
}
export function getEmitFn(): EmitFn | null {
  return _emitFn;
}

// Keep the existing array-style call sites in `index.ts` (`onReadyFns.push`
// / `onCloseFns.push`) working without exposing the internal Map<Set> bus.
type LifecycleEvent = "ready" | "close";
const _bus = new Map<LifecycleEvent, Set<() => void>>([
  ["ready", new Set()],
  ["close", new Set()],
]);

/**
 * Adapter that mimics the old `Array.push`-based API. Using the same
 * shape means the `app.onReady`/`app.onClose` call sites in `index.ts`
 * stay untouched.
 */
const arrayLike = (event: LifecycleEvent) => ({
  push(fn: () => void) {
    _bus.get(event)?.add(fn);
  },
});

export const onReadyFns = arrayLike("ready");
export const onCloseFns = arrayLike("close");

export const lifecycleFlags = { readyFired: false, closeFired: false };

/** Upper bound on how long shutdown waits for `app:beforeQuit` plugin hooks. */
export const BEFORE_QUIT_TIMEOUT_MS = 3000;

/**
 * Fire plugin `app:beforeStart` hooks with the validated app config. Awaited
 * by the runtime before the config frame is sent, so hooks run first.
 */
export function fireBeforeStart(config: unknown): Promise<void> {
  return fireHook("app:beforeStart", config);
}

/**
 * Fire plugin `app:beforeQuit` hooks. Bounded by a timeout so a hung hook
 * cannot keep the process alive once the host asked it to stop.
 */
export async function fireBeforeQuit(timeoutMs = BEFORE_QUIT_TIMEOUT_MS): Promise<void> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  const timeout = new Promise<void>((resolve) => {
    timer = setTimeout(resolve, timeoutMs);
  });
  try {
    await Promise.race([fireHook("app:beforeQuit"), timeout]);
  } finally {
    clearTimeout(timer);
  }
}

/**
 * Run one lifecycle handler, isolating it: a sync throw or an async
 * rejection is logged instead of blocking the other handlers or surfacing
 * as an unhandled rejection.
 */
export function runHandler(event: LifecycleEvent, fn: () => void): void {
  const report = (e: unknown) =>
    tynd.error(`app.on${event === "ready" ? "Ready" : "Close"} handler failed: ${e}`);
  try {
    const out = fn() as unknown;
    if (out && typeof (out as PromiseLike<unknown>).then === "function") {
      (out as PromiseLike<unknown>).then(undefined, report);
    }
  } catch (e) {
    report(e);
  }
}

/**
 * Fire every registered `app.onReady` handler; individual errors are logged.
 * User handlers run synchronously (as before); the returned promise settles
 * once plugin `app:ready` hooks have finished.
 */
export function fireReady(): Promise<void> {
  lifecycleFlags.readyFired = true;
  const pluginHooks = fireHook("app:ready");
  // Snapshot + isolate per-handler errors so one bad listener can't
  // block the others.
  const handlers = Array.from(_bus.get("ready") ?? []);
  _bus.get("ready")?.clear();
  for (const fn of handlers) runHandler("ready", fn);
  return pluginHooks;
}

/** Fire every registered `app.onClose` handler; individual errors are logged. */
export function fireClose(): void {
  lifecycleFlags.closeFired = true;
  const handlers = Array.from(_bus.get("close") ?? []);
  _bus.get("close")?.clear();
  for (const fn of handlers) runHandler("close", fn);
  // Drop any frontend-emit handlers so the listener map doesn't outlive
  // the app. User code that relies on its own unsubscribe pattern is
  // unaffected (that path already ran during normal teardown).
  clearFrontendListeners();
}
