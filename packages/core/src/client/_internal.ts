/**
 * Handle returned by `window.__tynd__.call`. Awaiting it resolves with the
 * handler's final return value (or the generator's return value for streams);
 * iterating it yields each chunk of a streaming (async-iterable) handler.
 */
export interface CallHandle<Y = unknown, R = unknown> extends PromiseLike<R>, AsyncIterable<Y> {
  cancel(): Promise<IteratorResult<Y>>
}

declare global {
  interface Window {
    __tynd__: {
      call(fn: string, args: unknown[]): CallHandle
      os_call(api: string, method: string, args: unknown): Promise<unknown>
      os_on(name: string, handler: (data: unknown) => void): () => void
      on(name: string, handler: (payload: unknown) => void): () => void
      off(name: string, handler: (payload: unknown) => void): void
    }
    __tynd_os_result__: (id: string, ok: boolean, value: unknown) => void
    __tynd_os_event__: (name: string, data: unknown) => void
    __tynd_yield__: (id: string, value: unknown) => void
  }
}

export function osCall<T>(api: string, method: string, args: unknown = null): Promise<T> {
  return window.__tynd__.os_call(api, method, args) as Promise<T>
}

export function bytesToBase64(bytes: Uint8Array): string {
  let s = ""
  for (let i = 0; i < bytes.length; i++) s += String.fromCharCode(bytes[i]!)
  return btoa(s)
}

export function base64ToBytes(b64: string): Uint8Array {
  const s = atob(b64)
  const out = new Uint8Array(s.length)
  for (let i = 0; i < s.length; i++) out[i] = s.charCodeAt(i)
  return out
}

const BIN_ORIGIN = "tynd-bin://localhost"

function buildBinUrl(path: string, query?: Record<string, string>): string {
  const qs = query
    ? "?" +
      Object.entries(query)
        .map(([k, v]) => `${encodeURIComponent(k)}=${encodeURIComponent(v)}`)
        .join("&")
    : ""
  return `${BIN_ORIGIN}/${path}${qs}`
}

async function throwIfError(r: Response, label: string): Promise<void> {
  if (r.ok) return
  const text = await r.text().catch(() => r.statusText)
  throw new Error(`${label}: ${text || r.statusText}`)
}

/** GET a binary buffer from the `tynd-bin://` IPC scheme. */
export async function binFetch(path: string, query?: Record<string, string>): Promise<Uint8Array> {
  const r = await fetch(buildBinUrl(path, query))
  await throwIfError(r, path)
  const buf = await r.arrayBuffer()
  return new Uint8Array(buf)
}

/** POST raw bytes to the `tynd-bin://` IPC scheme and receive raw bytes back. */
export async function binExchange(
  path: string,
  query: Record<string, string> | undefined,
  body: Uint8Array | ArrayBuffer,
): Promise<Uint8Array> {
  const bytes = body instanceof ArrayBuffer ? new Uint8Array(body) : body
  const r = await fetch(buildBinUrl(path, query), {
    method: "POST",
    body: bytes as BodyInit,
  })
  await throwIfError(r, path)
  const buf = await r.arrayBuffer()
  return new Uint8Array(buf)
}

/** POST raw bytes and receive a UTF-8 string (for APIs like `compute/hash`). */
export async function binExchangeText(
  path: string,
  query: Record<string, string> | undefined,
  body: Uint8Array | ArrayBuffer,
): Promise<string> {
  const bytes = body instanceof ArrayBuffer ? new Uint8Array(body) : body
  const r = await fetch(buildBinUrl(path, query), {
    method: "POST",
    body: bytes as BodyInit,
  })
  await throwIfError(r, path)
  return r.text()
}

/** POST raw bytes with no expected response body (writes). */
export async function binUpload(
  path: string,
  query: Record<string, string> | undefined,
  body: Uint8Array | ArrayBuffer,
): Promise<void> {
  const bytes = body instanceof ArrayBuffer ? new Uint8Array(body) : body
  const r = await fetch(buildBinUrl(path, query), {
    method: "POST",
    body: bytes as BodyInit,
  })
  await throwIfError(r, path)
}
