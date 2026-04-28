import { tynd } from "../logger";

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

      return (...args: unknown[]) => window.__tynd__.call(routeFn(prop), args);
    },
  });
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
