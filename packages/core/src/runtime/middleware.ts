/**
 * Backend middleware chain (contract B2).
 * `app.use(mw)` registers a function that runs around every user backend
 * function and every JS plugin method, in registration order (first
 * registered = outermost). A throw (or rejection) anywhere in the chain
 * rejects the call through the regular IPC error path.
 */

import type { CallContext } from "./context";
import { ipcHooksMiddleware } from "./hooks";

/** What a middleware sees about the call in flight. */
export interface MiddlewareContext {
  /** Exported function name, or the plugin method name when `plugin` is set. */
  fn: string;
  /** Decoded call arguments. Middleware may replace entries before `next()`. */
  args: unknown[];
  /** Identity of the calling webview; `undefined` when the host sent none. */
  caller: CallContext | undefined;
  /** Plugin name when the call targets a JS plugin method. */
  plugin?: string;
}

/**
 * Express/Koa-style middleware. Call `next()` to continue down the chain
 * and return (or transform) its result. Not calling `next()` short-circuits
 * the call with the value you return.
 */
export type Middleware = (
  ctx: MiddlewareContext,
  next: () => Promise<unknown>,
) => Promise<unknown> | unknown;

const stack: Middleware[] = [];

/** Append a middleware to the chain. */
export function useMiddleware(mw: Middleware): void {
  if (typeof mw !== "function") {
    throw new TypeError(`app.use(): middleware must be a function (got ${typeof mw})`);
  }
  stack.push(mw);
}

/** @internal Test helper: drop every registered middleware. */
export function clearMiddleware(): void {
  stack.length = 0;
}

/**
 * Run `final` wrapped in the chain: plugin `ipc:*` hooks first (outermost),
 * then user middleware in registration order.
 */
export function runMiddleware(
  ctx: MiddlewareContext,
  final: () => Promise<unknown> | unknown,
): Promise<unknown> {
  // Snapshot so a middleware registered mid-call cannot reshuffle this call.
  const chain: Middleware[] = [ipcHooksMiddleware, ...stack];
  let last = -1;
  const dispatch = async (i: number): Promise<unknown> => {
    if (i <= last) throw new Error("middleware: next() called more than once");
    last = i;
    const mw = chain[i];
    if (!mw) return final();
    return mw(ctx, () => dispatch(i + 1));
  };
  return dispatch(0);
}
