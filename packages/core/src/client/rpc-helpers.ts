/**
 * RPC timeout + AbortSignal helpers. Backend calls made through
 * `createBackend` are plain Promises — these utilities add deadline +
 * cancellation on top without changing the public API shape.
 */

import pTimeout from "p-timeout";

/**
 * Subscribe to a backend event; automatically unsubscribe after `n`
 * invocations. Returns a manual unsubscribe for the case where the
 * subscriber wants to stop early.
 */
export function listenN<T = unknown>(
  event: string,
  n: number,
  handler: (payload: T) => void,
): () => void {
  if (!Number.isFinite(n) || n < 1) {
    throw new RangeError(`listenN: \`n\` must be a positive integer, got ${n}`);
  }
  let remaining = n;
  const wrapper = (raw: unknown) => {
    handler(raw as T);
    remaining -= 1;
    if (remaining <= 0) {
      window.__tynd__.off(event, wrapper);
    }
  };
  window.__tynd__.on(event, wrapper);
  return () => window.__tynd__.off(event, wrapper);
}

/**
 * Subscribe to a backend event once, mirroring the EventEmitter pattern.
 * `createBackend()` already exposes `.once()` for typed events — this
 * helper is the untyped variant for ad-hoc events like `window:focused`.
 */
export function once<T = unknown>(event: string, handler: (payload: T) => void): () => void {
  return listenN<T>(event, 1, handler);
}

export class RpcTimeoutError extends Error {
  constructor(ms: number) {
    super(`RPC timed out after ${ms}ms`);
    this.name = "RpcTimeoutError";
  }
}

/**
 * Reject `promise` with `RpcTimeoutError` if it hasn't settled within
 * `ms`. Note: the timeout does not cancel the backend-side work for
 * non-streaming calls (Tynd's RPC doesn't surface mid-call cancellation
 * yet). Use streaming + `handle.cancel()` when true cancellation matters.
 *
 * Implemented on top of `p-timeout` so the rejection cleanup (clearing the
 * underlying timer when the promise settles first) is bullet-proof; the
 * rejection error is our `RpcTimeoutError` so existing `instanceof` checks
 * keep working.
 */
export function withTimeout<T>(promise: Promise<T>, ms: number): Promise<T> {
  return pTimeout(promise, { milliseconds: ms, message: new RpcTimeoutError(ms) });
}

/**
 * Race `promise` against an `AbortSignal`. The signal's `abort()` rejects
 * the returned promise with its `reason` (or a generic `AbortError`). Like
 * `withTimeout`, this doesn't propagate to the backend for non-streaming
 * calls; use streaming + `handle.cancel()` for real work-stoppage.
 */
export function abortable<T>(promise: Promise<T>, signal: AbortSignal): Promise<T> {
  if (signal.aborted) {
    return Promise.reject(signal.reason ?? new DOMException("Aborted", "AbortError"));
  }
  // p-timeout's `signal` option rejects with `signal.reason` when fired,
  // and `Infinity` disables the timeout side entirely so we get a pure
  // abort racer in ~3 lines instead of a manual addEventListener dance.
  return pTimeout(promise, { milliseconds: Number.POSITIVE_INFINITY, signal });
}
