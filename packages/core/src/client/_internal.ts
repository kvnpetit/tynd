import { TYND_IPC_ERR_SENTINEL, TyndIpcError, type TyndIpcErrorCode } from "../errors";
import { tynd } from "../logger";
import { encodeChannelArgs } from "./channel";

const KNOWN_IPC_ERROR_CODES: ReadonlySet<TyndIpcErrorCode> = new Set([
  "UNKNOWN_FN",
  "INVALID_ARGS",
  "HANDLER_THREW",
  "CANCELLED",
  "TIMEOUT",
  "PERMISSION_DENIED",
  "BACKEND_DEAD",
  "PROTOCOL_ERROR",
]);

/**
 * Decode an IPC error payload from the host shim. The shim wraps every
 * rejected call in `new Error(String(valueStr))`, so we receive an `Error`
 * whose `.message` MAY start with the `__TYND_IPC_ERR__:` sentinel produced
 * by the Bun runtime (`runtime/bun.ts::encodeIpcError`).
 * - Sentinel match: parse the JSON tail and reconstruct a `TyndIpcError`
 *   with the original `code` + `fn`. Falls back to HANDLER_THREW if the
 *   payload is malformed (defensive — shouldn't happen with our own runtime).
 * - `.code === "BACKEND_DEAD"` (set by the shim's backend-dead path): rebuilt as
 *   a `TyndIpcError` with that code.
 * - No sentinel: return the original error untouched. Plain `Error` from
 *   OS calls (`os_call`).
 */
export function decodeIpcError(err: unknown, fnHint?: string): unknown {
  if (!(err instanceof Error) || typeof err.message !== "string") return err;
  // The shim rejects in-flight calls with a plain `Error` carrying
  // `.code === "BACKEND_DEAD"` (no sentinel) when the Bun child exits.
  if ((err as { code?: unknown }).code === "BACKEND_DEAD") {
    return new TyndIpcError({ code: "BACKEND_DEAD", message: err.message, fn: fnHint, cause: err });
  }
  if (!err.message.startsWith(TYND_IPC_ERR_SENTINEL)) return err;
  const tail = err.message.slice(TYND_IPC_ERR_SENTINEL.length);
  let payload: { name?: unknown; code?: unknown; message?: unknown; fn?: unknown };
  try {
    payload = JSON.parse(tail) as typeof payload;
  } catch {
    // Malformed sentinel — surface as PROTOCOL_ERROR rather than swallowing
    // info; this is a runtime/transport bug, not a user error.
    return new TyndIpcError({
      code: "PROTOCOL_ERROR",
      message: `malformed IPC error envelope: ${tail.slice(0, 200)}`,
      fn: fnHint,
      cause: err,
    });
  }
  const code =
    typeof payload.code === "string" && KNOWN_IPC_ERROR_CODES.has(payload.code as TyndIpcErrorCode)
      ? (payload.code as TyndIpcErrorCode)
      : "HANDLER_THREW";
  const message = typeof payload.message === "string" ? payload.message : err.message;
  const fn = typeof payload.fn === "string" ? payload.fn : fnHint;
  return new TyndIpcError({ code, message, fn, cause: err.cause });
}

/**
 * Handle returned by `window.__tynd__.call`. Awaiting it resolves with the
 * handler's final return value (or the generator's return value for streams);
 * iterating it yields each chunk of a streaming (async-iterable) handler.
 */
export interface CallHandle<Y = unknown, R = unknown> extends PromiseLike<R>, AsyncIterable<Y> {
  cancel(): Promise<IteratorResult<Y>>;
}

declare global {
  interface Window {
    __tynd__: {
      call(fn: string, args: unknown[]): CallHandle;
      os_call(api: string, method: string, args: unknown, opts?: OsCallOptions): Promise<unknown>;
      os_on(name: string, handler: (data: unknown) => void): () => void;
      on(name: string, handler: (payload: unknown) => void): () => void;
      off(name: string, handler: (payload: unknown) => void): void;
    };
    __tynd_os_result__: (id: string, ok: boolean, value: unknown) => void;
    __tynd_os_event__: (name: string, data: unknown) => void;
    __tynd_yield__: (id: string, value: unknown) => void;
    __TYND_WINDOW_LABEL__?: string;
    __TYND_WEBVIEW_LABEL__?: string;
  }
}

/** Label of the current window. Defaults to `"main"` for the primary window. */
export function currentWindowLabel(): string {
  return typeof window === "undefined" ? "main" : (window.__TYND_WINDOW_LABEL__ ?? "main");
}

/** Label of the current webview. Equals the window label for top-level webviews. */
export function currentWebviewLabel(): string {
  return typeof window === "undefined"
    ? "main"
    : (window.__TYND_WEBVIEW_LABEL__ ?? currentWindowLabel());
}

/** Per-call options for `window.__tynd__.os_call`. */
export interface OsCallOptions {
  /**
   * Reject after this many milliseconds. Omitted by default: OS calls have no
   * deadline (dialogs wait on the user, downloads and process runs can take
   * minutes). The host keeps working after the client gives up.
   */
  timeoutMs?: number;
}

/** Slack added to a caller's `timeoutMs` so the host's own (more informative) timeout error wins the race. */
const OS_TIMEOUT_GRACE_MS = 5_000;

/**
 * Client-side deadline for an OS call whose request carries its own
 * `timeoutMs`; `undefined` (no deadline) when the caller passed none.
 */
export function osDeadline(timeoutMs: unknown): OsCallOptions | undefined {
  return typeof timeoutMs === "number" && timeoutMs > 0
    ? { timeoutMs: timeoutMs + OS_TIMEOUT_GRACE_MS }
    : undefined;
}

export function osCall<T>(
  api: string,
  method: string,
  args: unknown = null,
  opts?: OsCallOptions,
): Promise<T> {
  const p = (
    opts
      ? window.__tynd__.os_call(api, method, args, opts)
      : window.__tynd__.os_call(api, method, args)
  ) as Promise<T>;
  // Surface the shim's BACKEND_DEAD rejection as a typed `TyndIpcError`.
  return p.then(undefined, (e: unknown) => {
    throw decodeIpcError(e);
  });
}

/**
 * Subscribe to a host-emitted OS event (`menu:action`, `fs:change`, ...).
 * Returns the unsubscribe function. The payload type is the caller's claim:
 * the host sends it untyped.
 */
export function osOn<T = unknown>(name: string, handler: (payload: T) => void): () => void {
  return window.__tynd__.os_on(name, handler as (data: unknown) => void);
}

// Prefer the TC39-stage-3 `Uint8Array.prototype.toBase64` / `fromBase64`
// helpers when available (Bun >= 1.1.20, Chromium >= 133). They short-circuit
// the chunked `String.fromCharCode` / `btoa` dance below — relevant on hot
// FS / WebSocket / HTTP paths where every roundtrip touches base64. For older
// WebView2 runtimes (Chromium 120-132 still ship widely on Windows in 2026)
// we keep a 3-line `atob`/`btoa` fallback that's binary-safe up to ~64 KiB.
const _u8 = Uint8Array as unknown as {
  fromBase64?: (b64: string) => Uint8Array;
  prototype: { toBase64?: () => string };
};
const _nativeFromB64 = typeof _u8.fromBase64 === "function" ? _u8.fromBase64 : null;
const _hasNativeB64 = _nativeFromB64 !== null && typeof _u8.prototype.toBase64 === "function";

// Chunk size picked so `String.fromCharCode.apply` stays under typical
// argument-length limits (~65k) while keeping O(n) overall cost.
const B64_CHUNK = 0x8000;

export function bytesToBase64(bytes: Uint8Array): string {
  if (bytes.length === 0) return "";
  if (_hasNativeB64) return (bytes as unknown as { toBase64(): string }).toBase64();
  const parts: string[] = [];
  for (let i = 0; i < bytes.length; i += B64_CHUNK) {
    const slice = bytes.subarray(i, Math.min(i + B64_CHUNK, bytes.length));
    parts.push(String.fromCharCode.apply(null, slice as unknown as number[]));
  }
  return btoa(parts.join(""));
}

export function base64ToBytes(b64: string): Uint8Array {
  if (_nativeFromB64) return _nativeFromB64(b64);
  const s = atob(b64);
  const out = new Uint8Array(s.length);
  for (let i = 0; i < s.length; i++) out[i] = s.charCodeAt(i);
  return out;
}

/**
 * Centralizes the proxy traps shared by createBackend and createPluginClient.
 * `routeFn` maps the JS property name to the wire fn name — backend uses
 * the raw prop, plugins prefix with `$plugin$<name>$`.
 */
export function createCallProxy<T extends object>(
  routeFn: (prop: string) => string,
  extraTraps?: (prop: string) => unknown,
  callOptions?: CallOptions,
): T {
  return new Proxy({} as T, {
    get(_target, prop: string | symbol) {
      if (typeof prop !== "string") return;
      // Block thenable traps so `await proxy` doesn't fire a stray call.
      if (prop === "then" || prop === "catch" || prop === "finally") return;

      if (typeof window === "undefined" || !window.__tynd__) {
        tynd.error("window.__tynd__ is not available — are you running outside a tynd app?");
        return () => Promise.reject(new Error("[tynd] not in a tynd app context"));
      }

      const extra = extraTraps?.(prop);
      if (extra !== undefined) return extra;

      return (...args: unknown[]) => {
        const wireFn = routeFn(prop);
        const handle = window.__tynd__.call(wireFn, encodeChannelArgs(args));
        // Return a new handle that intercepts both the awaited final value
        // and the async-iterable yields, transparently rethrowing host
        // errors as `TyndIpcError` when the wire envelope is present.
        const wrapped = wrapCallHandleErrors(handle, wireFn);
        return callOptions ? applyCallOptions(wrapped, wireFn, callOptions) : wrapped;
      };
    },
  });
}

/** Per-call options accepted by `api.withOptions(...)`. */
export interface CallOptions {
  /**
   * Cancel the call when this signal aborts: the promise (and a stream's
   * iteration) rejects with `signal.reason` (an `AbortError` by default) and
   * the backend handler's `getCallContext().signal` aborts.
   */
  signal?: AbortSignal;
  /**
   * Cancel the call after this many milliseconds: the promise rejects with a
   * `TyndIpcError` of code `TIMEOUT`. No deadline by default.
   */
  timeout?: number;
}

/** Validate `withOptions(...)` input; a typo must fail loudly, not silently drop the deadline. */
export function checkCallOptions(opts: unknown): CallOptions {
  if (opts === null || typeof opts !== "object") {
    throw new TypeError("withOptions(): expected an object like { signal, timeout }");
  }
  const { signal, timeout } = opts as Record<string, unknown>;
  if (
    signal !== undefined &&
    (signal === null ||
      typeof (signal as AbortSignal).addEventListener !== "function" ||
      typeof (signal as AbortSignal).aborted !== "boolean")
  ) {
    throw new TypeError("withOptions(): signal must be an AbortSignal");
  }
  if (
    timeout !== undefined &&
    (typeof timeout !== "number" || !Number.isFinite(timeout) || timeout <= 0)
  ) {
    throw new TypeError("withOptions(): timeout must be a positive number of milliseconds");
  }
  return { signal: signal as AbortSignal | undefined, timeout: timeout as number | undefined };
}

/**
 * Bind an abort signal and/or a deadline to a call handle. Either one cancels
 * the call (`handle.cancel()`: the host forwards the cancel, the backend
 * aborts the handler's signal) and replaces the shim's `CANCELLED` rejection
 * with the abort reason or a `TIMEOUT` error.
 */
function applyCallOptions(
  handle: CallHandle & Promise<unknown>,
  fn: string,
  opts: CallOptions,
): CallHandle & Promise<unknown> {
  const { signal, timeout } = opts;
  if (!signal && timeout === undefined) return handle;
  let reason: unknown;
  let timer: ReturnType<typeof setTimeout> | undefined;
  const cleanup = (): void => {
    clearTimeout(timer);
    signal?.removeEventListener("abort", onAbort);
  };
  const stop = (why: unknown): void => {
    if (reason !== undefined) return;
    reason = why;
    cleanup();
    handle.cancel().catch(() => undefined);
  };
  function onAbort(): void {
    stop(signal?.reason ?? new DOMException("call aborted", "AbortError"));
  }
  const mapReject = (error: unknown): never => {
    throw reason ?? error;
  };
  const promise: Promise<unknown> = Promise.resolve(handle).then(
    (value) => {
      cleanup();
      return value;
    },
    (error: unknown) => {
      cleanup();
      return mapReject(error);
    },
  );
  const markHandled = (): void => {
    promise.catch(() => undefined);
  };

  if (signal?.aborted) {
    onAbort();
  } else {
    signal?.addEventListener("abort", onAbort, { once: true });
    if (timeout !== undefined) {
      timer = setTimeout(() => stop(timeoutError(fn, timeout)), timeout);
    }
  }

  const settle = (): void => {
    cleanup();
    markHandled();
  };
  return Object.assign(promise, {
    cancel: (): Promise<IteratorResult<unknown>> => {
      settle();
      return handle.cancel();
    },
    [Symbol.asyncIterator]: () => {
      markHandled();
      return optionIterator(handle, mapReject, settle);
    },
  }) as CallHandle & Promise<unknown>;
}

/** Iterate `handle`, mapping rejections through `mapReject`; `onReturn` runs on early exit. */
function optionIterator(
  handle: CallHandle,
  mapReject: (error: unknown) => never,
  onReturn: () => void,
): AsyncIterator<unknown> & AsyncIterable<unknown> {
  const inner = handle[Symbol.asyncIterator]();
  const wrap = <R>(p: Promise<R>): Promise<R> => p.then(undefined, mapReject) as Promise<R>;
  const iterator: AsyncIterator<unknown> & AsyncIterable<unknown> = {
    next: () => wrap(inner.next()),
    return: (value?: unknown) => {
      onReturn();
      return wrap(inner.return ? inner.return(value) : Promise.resolve({ value, done: true }));
    },
    [Symbol.asyncIterator]() {
      return iterator;
    },
  };
  return iterator;
}

function timeoutError(fn: string, timeout: number): TyndIpcError {
  return new TyndIpcError({
    code: "TIMEOUT",
    message: `call '${fn}' timed out after ${timeout}ms`,
    fn,
  });
}

/**
 * Wrap a `CallHandle` so any rejection (final await OR per-chunk iteration)
 * surfaces as `TyndIpcError` when the host's error message carries the
 * sentinel envelope. Plain rejections (OS calls)
 * pass through untouched — `decodeIpcError` is a no-op for those.
 * The result is a real `Promise` (so `.then` / `.catch` / `.finally` all
 * work) with `cancel()` and `[Symbol.asyncIterator]` attached.
 */
function wrapCallHandleErrors(handle: CallHandle, fn: string): CallHandle & Promise<unknown> {
  // Every rejection (the awaited final value AND the iterator's `next()` /
  // `return()` / `throw()`) is funnelled through `decodeIpcError` so a
  // streaming handler that rejects mid-stream still surfaces as a
  // typed `TyndIpcError` instead of plain `Error`. Resolved settles are
  // forwarded as-is; only rejected settles pay the `decodeIpcError` hop.
  const decodeReject = (reason: unknown): never => {
    throw decodeIpcError(reason, fn);
  };
  const promise: Promise<unknown> = Promise.resolve(handle).then(undefined, decodeReject);
  // An explicit cancel, a `break` out of a `for await`, or any iteration at
  // all (errors surface via `next()`) leaves the final promise unawaited, so
  // mark it handled. Awaiting it after a cancel still observes the rejection.
  const markHandled = (): void => {
    promise.catch(() => undefined);
  };

  const wrapAsyncIterator = () => {
    // A consumer iterating owns error delivery: failures reach it through
    // `next()`, so the eager final promise must not also raise an
    // `unhandledrejection` for an error the loop already caught.
    markHandled();
    const inner = handle[Symbol.asyncIterator]();
    const wrapErr = <T>(p: Promise<T>): Promise<T> => p.then(undefined, decodeReject) as Promise<T>;
    const next = inner.next.bind(inner);
    const ret = inner.return?.bind(inner);
    const thr = inner.throw?.bind(inner);
    return {
      next: (...args: [] | [unknown]) => wrapErr(next(...(args as [unknown]))),
      ...(ret
        ? {
            return: (value?: unknown) => {
              markHandled();
              return wrapErr(ret(value));
            },
          }
        : {}),
      ...(thr ? { throw: (e?: unknown) => wrapErr(thr(e)) } : {}),
      [Symbol.asyncIterator](): AsyncIterator<unknown> {
        return this;
      },
    };
  };

  return Object.assign(promise, {
    cancel: (): Promise<IteratorResult<unknown>> => {
      markHandled();
      return handle.cancel().catch(decodeReject as (r: unknown) => never);
    },
    [Symbol.asyncIterator]: wrapAsyncIterator,
  }) as CallHandle & Promise<unknown>;
}

/**
 * Origin of the `tynd-bin://` IPC scheme. WebView2 (Windows) and Android WebView cannot
 * navigate to custom schemes: wry serves them under `http://<scheme>.localhost` there.
 * The host injects the exact origin as `window.__TYND_BIN_ORIGIN__`; the user-agent guess is only
 * a fallback because an app can override `navigator.userAgent` (`window.userAgent`).
 */
export function binOrigin(userAgent: string | undefined = globalThis.navigator?.userAgent): string {
  const injected = (globalThis as { __TYND_BIN_ORIGIN__?: unknown }).__TYND_BIN_ORIGIN__;
  if (typeof injected === "string" && injected) return injected;
  return /Windows|Android/i.test(userAgent ?? "")
    ? "http://tynd-bin.localhost"
    : "tynd-bin://localhost";
}

function buildBinUrl(path: string, query?: Record<string, string>): string {
  const qs = query
    ? "?" +
      Object.entries(query)
        .map(([k, v]) => `${encodeURIComponent(k)}=${encodeURIComponent(v)}`)
        .join("&")
    : "";
  return `${binOrigin()}/${path}${qs}`;
}

async function throwIfError(r: Response, label: string): Promise<void> {
  if (r.ok) return;
  const text = await r.text().catch(() => r.statusText);
  throw new Error(`${label}: ${text || r.statusText}`);
}

type BinResponse = "bytes" | "text" | "void";
type BinReturn<R extends BinResponse> = R extends "bytes"
  ? Uint8Array
  : R extends "text"
    ? string
    : undefined;

interface BinCallOpts<R extends BinResponse> {
  path: string;
  query?: Record<string, string>;
  body?: Uint8Array | ArrayBuffer;
  response: R;
}

// Deadline for the host to start answering a `tynd-bin://` round-trip. A
// wedged or restarting host would otherwise leave every fs/compute renderer
// call parked forever — no error, no UI feedback. It covers time-to-headers
// only (cleared once the response starts, so a large read that is making
// progress is never aborted) and grows by 1s per MiB of uploaded body, since
// the host answers after it has consumed the whole request.
const BIN_DEFAULT_TIMEOUT_MS = 30_000;
const BIN_TIMEOUT_PER_MIB_MS = 1_000;

function binDeadline(body: Uint8Array | ArrayBuffer | undefined): number {
  const bytes = body ? body.byteLength : 0;
  return BIN_DEFAULT_TIMEOUT_MS + Math.ceil(bytes / (1024 * 1024)) * BIN_TIMEOUT_PER_MIB_MS;
}

/**
 * Single-entry IPC call into the `tynd-bin://` scheme: `opts.response` picks
 * the return shape and `opts.body` picks GET vs POST.
 */
async function binCall<R extends BinResponse>(opts: BinCallOpts<R>): Promise<BinReturn<R>> {
  const ac = new AbortController();
  const timer = setTimeout(() => ac.abort(), binDeadline(opts.body));
  try {
    const init: RequestInit =
      opts.body !== undefined
        ? {
            method: "POST",
            body: (opts.body instanceof ArrayBuffer
              ? new Uint8Array(opts.body)
              : opts.body) as BodyInit,
            signal: ac.signal,
          }
        : { signal: ac.signal };
    const r = await fetch(buildBinUrl(opts.path, opts.query), init);
    clearTimeout(timer);
    await throwIfError(r, opts.path);
    if (opts.response === "bytes") {
      const buf = await r.arrayBuffer();
      return new Uint8Array(buf) as BinReturn<R>;
    }
    if (opts.response === "text") return (await r.text()) as BinReturn<R>;
    return undefined as BinReturn<R>;
  } finally {
    clearTimeout(timer);
  }
}

/** GET a binary buffer from the `tynd-bin://` IPC scheme. */
export function binFetch(path: string, query?: Record<string, string>): Promise<Uint8Array> {
  return binCall({ path, query, response: "bytes" });
}

/** POST raw bytes and receive a UTF-8 string (for APIs like `compute/hash`). */
export function binExchangeText(
  path: string,
  query: Record<string, string> | undefined,
  body: Uint8Array | ArrayBuffer,
): Promise<string> {
  return binCall({ path, query, body, response: "text" });
}

/** POST raw bytes with no expected response body (writes). */
export function binUpload(
  path: string,
  query: Record<string, string> | undefined,
  body: Uint8Array | ArrayBuffer,
): Promise<void> {
  return binCall({ path, query, body, response: "void" });
}
