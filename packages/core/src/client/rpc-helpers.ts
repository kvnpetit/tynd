/**
 * RPC timeout + AbortSignal helpers. Backend calls made through
 * `createBackend` are plain Promises — these utilities add deadline +
 * cancellation on top without changing the public API shape.
 */
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

/**
 * Discriminable timeout error — `code: "TIMEOUT"` lets handlers
 * pattern-match on the failure reason without `instanceof` dance.
 * Mirrors the `.code` field on `TyndIpcError` for consistency across
 * the client surface.
 */
export class RpcTimeoutError extends Error {
  readonly code = "TIMEOUT" as const;
  readonly timeoutMs: number;
  constructor(ms: number) {
    super(`RPC timed out after ${ms}ms`);
    this.name = "RpcTimeoutError";
    this.timeoutMs = ms;
  }
}

/**
 * Reject `promise` with `RpcTimeoutError` if it hasn't settled within
 * `ms`. Note: the timeout does not cancel the backend-side work for
 * non-streaming calls (Tynd's RPC doesn't surface mid-call cancellation
 * yet). Use streaming + `handle.cancel()` when true cancellation matters.
 * The rejection error is our `RpcTimeoutError` so existing `instanceof`
 * checks keep working.
 */
export function withTimeout<T>(promise: Promise<T>, ms: number): Promise<T> {
  return new Promise<T>((resolve, reject) => {
    const timer = setTimeout(() => {
      reject(new RpcTimeoutError(ms));
    }, ms);
    promise.then(
      (value) => {
        clearTimeout(timer);
        resolve(value);
      },
      (error) => {
        clearTimeout(timer);
        reject(error);
      },
    );
  });
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
  return new Promise<T>((resolve, reject) => {
    const onAbort = () => {
      reject(signal.reason ?? new DOMException("Aborted", "AbortError"));
    };
    signal.addEventListener("abort", onAbort, { once: true });
    promise.then(
      (value) => {
        signal.removeEventListener("abort", onAbort);
        resolve(value);
      },
      (error) => {
        signal.removeEventListener("abort", onAbort);
        reject(error);
      },
    );
  });
}
