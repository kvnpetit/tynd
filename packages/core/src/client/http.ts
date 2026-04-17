import { base64ToBytes, osCall } from "./_internal.js"

export interface HttpResponse<T = string> {
  status: number
  statusText: string
  headers: Record<string, string>
  body: T
}

export interface HttpProgress {
  phase: "upload" | "download"
  loaded: number
  total: number | null
}

export interface HttpRequestOptions {
  method?: "GET" | "POST" | "PUT" | "PATCH" | "DELETE" | "HEAD" | "OPTIONS"
  headers?: Record<string, string>
  body?: string | Record<string, unknown> | unknown[]
  timeoutMs?: number
  onProgress?: (p: HttpProgress) => void
}

async function run<T>(
  method: "request" | "download",
  baseArgs: Record<string, unknown>,
  onProgress?: (p: HttpProgress) => void,
): Promise<T> {
  if (!onProgress) return osCall<T>("http", method, baseArgs)
  const progressId = `http_${Date.now().toString(36)}_${Math.random().toString(36).slice(2, 8)}`
  const unsub = window.__tynd__.os_on("http:progress", (payload: unknown) => {
    const p = payload as HttpProgress & { id: string }
    if (p?.id === progressId) onProgress({ phase: p.phase, loaded: p.loaded, total: p.total })
  })
  try {
    return await osCall<T>("http", method, { ...baseArgs, progressId })
  } finally {
    unsub()
  }
}

export const http = {
  get<T = string>(
    url: string,
    opts?: Omit<HttpRequestOptions, "method" | "body">,
  ): Promise<HttpResponse<T>> {
    const { onProgress, ...rest } = opts ?? {}
    return run("request", { url, method: "GET", responseType: "text", ...rest }, onProgress)
  },
  getJson<T = unknown>(
    url: string,
    opts?: Omit<HttpRequestOptions, "method" | "body">,
  ): Promise<HttpResponse<T>> {
    const { onProgress, ...rest } = opts ?? {}
    return run("request", { url, method: "GET", responseType: "json", ...rest }, onProgress)
  },
  async getBinary(
    url: string,
    opts?: Omit<HttpRequestOptions, "method" | "body">,
  ): Promise<HttpResponse<Uint8Array>> {
    const { onProgress, ...rest } = opts ?? {}
    const res = await run<HttpResponse<string>>(
      "request",
      { url, method: "GET", responseType: "binary", ...rest },
      onProgress,
    )
    return { ...res, body: base64ToBytes(res.body) }
  },
  post<T = string>(url: string, opts?: HttpRequestOptions): Promise<HttpResponse<T>> {
    const { onProgress, ...rest } = opts ?? {}
    return run("request", { url, method: "POST", responseType: "text", ...rest }, onProgress)
  },
  request<T = string>(url: string, opts?: HttpRequestOptions): Promise<HttpResponse<T>> {
    const { onProgress, ...rest } = opts ?? {}
    return run("request", { url, responseType: "text", ...rest }, onProgress)
  },
  download(
    url: string,
    dest: string,
    opts?: {
      headers?: Record<string, string>
      timeoutMs?: number
      onProgress?: (p: HttpProgress) => void
    },
  ): Promise<{ path: string; bytes: number }> {
    const { onProgress, ...rest } = opts ?? {}
    return run("download", { url, dest, ...rest }, onProgress)
  },
}
