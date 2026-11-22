export interface WaitForOptions {
  /** Give up after this many milliseconds (default 5000). */
  timeout?: number;
  /** Delay between attempts in milliseconds (default 50). */
  interval?: number;
  /** Message used in the timeout error. */
  message?: string;
}

/** Thrown by {@link waitFor} when the predicate never became truthy. */
export class WaitForTimeoutError extends Error {
  readonly lastError: unknown;

  constructor(message: string, lastError?: unknown) {
    super(message);
    this.name = "WaitForTimeoutError";
    this.lastError = lastError;
  }
}

/**
 * Poll `predicate` until it returns a truthy value and resolve with that value. Exceptions thrown
 * by the predicate count as "not yet" and are reported in the timeout error.
 */
export async function waitFor<T>(
  predicate: () => T | Promise<T>,
  options: WaitForOptions = {},
): Promise<Exclude<Awaited<T>, false | null | undefined | 0 | "">> {
  const timeout = options.timeout ?? 5_000;
  const interval = options.interval ?? 50;
  const deadline = Date.now() + timeout;
  let lastError: unknown;
  for (;;) {
    try {
      const value = await predicate();
      if (value) return value as Exclude<Awaited<T>, false | null | undefined | 0 | "">;
      lastError = undefined;
    } catch (error) {
      lastError = error;
    }
    if (Date.now() >= deadline) break;
    await new Promise((resolve) => setTimeout(resolve, interval));
  }
  const reason = lastError instanceof Error ? `: ${lastError.message}` : "";
  throw new WaitForTimeoutError(
    `${options.message ?? "waitFor condition"} not met within ${timeout} ms${reason}`,
    lastError,
  );
}
