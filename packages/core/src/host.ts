/**
 * Trusted Bun-backend bridge to the native host.
 *
 * This surface is intentionally unavailable in the WebView. It exists for
 * pure TypeScript plugins that still need one narrow native primitive (for
 * example a filesystem policy check before opening SQLite with `bun:sqlite`).
 */

import { asError } from "./error-utils";

type HostCallTransport = (frame: {
  type: "host_call";
  id: string;
  api: string;
  method: string;
  args: unknown;
}) => void;

interface PendingHostCall {
  resolve(value: unknown): void;
  reject(error: Error): void;
  timer?: ReturnType<typeof setTimeout>;
}

/** Options for {@link hostCall}. */
export interface HostCallOptions {
  /**
   * Reject after this many milliseconds so a lost `host_result` cannot hang
   * the caller forever. Defaults to 5 minutes; `0` disables the deadline.
   */
  timeoutMs?: number;
}

const HOST_CALL_DEFAULT_TIMEOUT_MS = 5 * 60_000;

const MAX_PENDING_HOST_CALLS = 4096;
const pending = new Map<string, PendingHostCall>();
let transport: HostCallTransport | null = null;
let sequence = 0;

function rejectPending(error: Error): void {
  for (const call of pending.values()) {
    clearTimeout(call.timer);
    call.reject(error);
  }
  pending.clear();
}

export function hostCall<T>(
  api: string,
  method: string,
  args: unknown = null,
  opts?: HostCallOptions,
): Promise<T> {
  const activeTransport = transport;
  if (!activeTransport) {
    return Promise.reject(
      new Error("hostCall() is only available after app.start() in the trusted Bun backend"),
    );
  }
  if (pending.size >= MAX_PENDING_HOST_CALLS) {
    return Promise.reject(new Error("hostCall(): too many pending native calls"));
  }
  const id = `host-${process.pid}-${++sequence}`;
  return new Promise<T>((resolve, reject) => {
    const entry: PendingHostCall = { resolve: resolve as (value: unknown) => void, reject };
    const timeoutMs = opts?.timeoutMs ?? HOST_CALL_DEFAULT_TIMEOUT_MS;
    if (timeoutMs > 0) {
      entry.timer = setTimeout(() => {
        if (!pending.delete(id)) return;
        reject(new Error(`hostCall(${api}.${method}) timed out after ${timeoutMs}ms`));
      }, timeoutMs);
    }
    pending.set(id, entry);
    try {
      activeTransport({ type: "host_call", id, api, method, args });
    } catch (error) {
      clearTimeout(entry.timer);
      pending.delete(id);
      reject(asError(error));
    }
  });
}

/** @internal Runtime wiring; not exported from the package root. */
export function setHostCallTransport(next: HostCallTransport | null): void {
  if (transport === next) return;
  if (transport && pending.size > 0) {
    rejectPending(new Error("native host bridge replaced"));
  }
  transport = next;
  if (next) return;
  rejectPending(new Error("native host bridge closed"));
}

/** @internal Resolve a native response received on Bun stdin. */
export function resolveHostCall(id: string, ok: boolean, value: unknown): void {
  const call = pending.get(id);
  if (!call) return;
  pending.delete(id);
  clearTimeout(call.timer);
  if (ok) call.resolve(value);
  else call.reject(asError(value, "native host call failed"));
}
