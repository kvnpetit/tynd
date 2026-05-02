import { TYND_IPC_ERR_SENTINEL, TyndIpcError, type TyndIpcErrorCode } from "../errors";
import { tynd } from "../logger";

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
 * by the full runtime (`runtime/full.ts::encodeIpcError`).
 *
 * - Sentinel match: parse the JSON tail and reconstruct a `TyndIpcError`
 *   with the original `code` + `fn`. Falls back to HANDLER_THREW if the
 *   payload is malformed (defensive — shouldn't happen with our own runtime).
 * - No sentinel: return the original error untouched. Plain `Error` from
 *   the lite runtime, OS calls (`os_call`), or pre-W7.1 backends.
 */
export function decodeIpcError(err: unknown, fnHint?: string): unknown {
  if (!(err instanceof Error) || typeof err.message !== "string") return err;
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
      os_call(api: string, method: string, args: unknown): Promise<unknown>;
      os_on(name: string, handler: (data: unknown) => void): () => void;
      on(name: string, handler: (payload: unknown) => void): () => void;
      off(name: string, handler: (payload: unknown) => void): void;
    };
    __tynd_os_result__: (id: string, ok: boolean, value: unknown) => void;
    __tynd_os_event__: (name: string, data: unknown) => void;
    __tynd_yield__: (id: string, value: unknown) => void;
  }
}

export function osCall<T>(api: string, method: string, args: unknown = null): Promise<T> {
  return window.__tynd__.os_call(api, method, args) as Promise<T>;
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
 * Shared proxy factory for the typed `createBackend` / `createPluginClient`
 * helpers. The two callers used to duplicate ~25 LoC of "block thenable
 * traps + guard window.__tynd__ + return a curried `call`" boilerplate;
 * collapsing it here keeps the surface (and the error message) consistent.
 *
 * `routeFn` maps the JS property name to the wire fn name — backend uses
 * the raw prop, plugins prefix with `$plugin$<name>$`.
 */
export function createCallProxy<T extends object>(
  routeFn: (prop: string) => string,
  extraTraps?: (prop: string) => unknown,
): T {
  return new Proxy({} as T, {
    get(_target, prop: string | symbol) {
      if (typeof prop !== "string") return undefined;
      // Block thenable traps so `await proxy` doesn't fire a stray call.
      if (prop === "then" || prop === "catch" || prop === "finally") return undefined;

      if (!window.__tynd__) {
        tynd.error("window.__tynd__ is not available — are you running outside a tynd app?");
        return () => Promise.reject(new Error("[tynd] not in a tynd app context"));
      }

      const extra = extraTraps?.(prop);
      if (extra !== undefined) return extra;

      return (...args: unknown[]) => {
        const wireFn = routeFn(prop);
        const handle = window.__tynd__.call(wireFn, args);
        // Return a new handle that intercepts both the awaited final value
        // and the async-iterable yields, transparently rethrowing host
        // errors as `TyndIpcError` when the wire envelope is present.
        return wrapCallHandleErrors(handle, wireFn);
      };
    },
  });
}

/**
 * Wrap a `CallHandle` so any rejection (final await OR per-chunk iteration)
 * surfaces as `TyndIpcError` when the host's error message carries the
 * sentinel envelope. Plain rejections (lite runtime, OS calls, legacy)
 * pass through untouched — `decodeIpcError` is a no-op for those.
 *
 * The wrapper preserves `cancel()` and the awaitable + async-iterable
 * shape; consumers can't tell the difference from a raw handle.
 */
function wrapCallHandleErrors(handle: CallHandle, fn: string): CallHandle {
  // Iteration is delegated 1:1 to the underlying handle's iterator —
  // wrapping `next()` would insert an extra microtask hop between the
  // consumer's `await` and the shim's chunk delivery, which is enough
  // to let the host pre-buffer chunks past a `cancel()` point. Iterator
  // errors therefore surface as plain `Error` (the sentinel still
  // travels in `.message`); the awaited final-value path below is the
  // primary `TyndIpcError` reconstruction site.
  const decodeReject = (reason: unknown): never => {
    throw decodeIpcError(reason, fn);
  };

  const wrapped: CallHandle = {
    // biome-ignore lint/suspicious/noThenProperty: mirrors CallHandle's intentional PromiseLike shape
    then<TResult1 = unknown, TResult2 = never>(
      onfulfilled?: ((value: unknown) => TResult1 | PromiseLike<TResult1>) | null | undefined,
      onrejected?: ((reason: unknown) => TResult2 | PromiseLike<TResult2>) | null | undefined,
    ): PromiseLike<TResult1 | TResult2> {
      const rejector: (r: unknown) => TResult2 | PromiseLike<TResult2> = onrejected
        ? (reason) => onrejected(decodeIpcError(reason, fn))
        : (decodeReject as unknown as (r: unknown) => TResult2);
      return Promise.resolve(handle).then(onfulfilled ?? undefined, rejector);
    },
    cancel: () => handle.cancel(),
    [Symbol.asyncIterator]: () => handle[Symbol.asyncIterator](),
  };
  return wrapped;
}

const BIN_ORIGIN = "tynd-bin://localhost";

function buildBinUrl(path: string, query?: Record<string, string>): string {
  const qs = query
    ? "?" +
      Object.entries(query)
        .map(([k, v]) => `${encodeURIComponent(k)}=${encodeURIComponent(v)}`)
        .join("&")
    : "";
  return `${BIN_ORIGIN}/${path}${qs}`;
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

/**
 * Single-entry IPC call into the `tynd-bin://` scheme. Replaces the four
 * near-identical helpers (`binFetch` / `binExchange` / `binExchangeText` /
 * `binUpload`) by switching on `opts.response` for the return shape and
 * `opts.body` for GET vs POST.
 */
async function binCall<R extends BinResponse>(opts: BinCallOpts<R>): Promise<BinReturn<R>> {
  const init: RequestInit | undefined =
    opts.body !== undefined
      ? {
          method: "POST",
          body: (opts.body instanceof ArrayBuffer
            ? new Uint8Array(opts.body)
            : opts.body) as BodyInit,
        }
      : undefined;
  const r = await fetch(buildBinUrl(opts.path, opts.query), init);
  await throwIfError(r, opts.path);
  if (opts.response === "bytes") {
    const buf = await r.arrayBuffer();
    return new Uint8Array(buf) as BinReturn<R>;
  }
  if (opts.response === "text") return (await r.text()) as BinReturn<R>;
  return undefined as BinReturn<R>;
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
