import { base64ToBytes, osCall, osDeadline, osOn } from "@tynd/core/client/bridge";

/** Generic HTTP response envelope returned by every `http.*` call. */
export interface HttpResponse<T = string> {
  status: number;
  statusText: string;
  headers: Record<string, string>;
  body: T;
}

/** Progress event emitted during an in-flight upload or download. */
export interface HttpProgress {
  phase: "upload" | "download";
  loaded: number;
  total: number | null;
}

/** Per-call options for `http.request` and friends. */
export interface HttpRequestOptions {
  method?: "GET" | "POST" | "PUT" | "PATCH" | "DELETE" | "HEAD" | "OPTIONS";
  headers?: Record<string, string>;
  body?: string | Record<string, unknown> | unknown[];
  timeoutMs?: number;
  onProgress?: (p: HttpProgress) => void;
}

async function run<T>(
  method: "request" | "download",
  baseArgs: Record<string, unknown>,
  onProgress?: (p: HttpProgress) => void,
): Promise<T> {
  if (!onProgress) return osCall<T>("http", method, baseArgs, osDeadline(baseArgs.timeoutMs));
  const progressId = `http_${Date.now().toString(36)}_${Math.random().toString(36).slice(2, 8)}`;
  // Subscribe BEFORE the await — even if `osCall` throws synchronously,
  // the `try/finally` guarantees `unsub()` runs. The `unsubbed` latch
  // makes a double-unsubscribe (caller cancels mid-flight, then finally
  // fires) idempotent so the host doesn't see two `os_off` for the
  // same id.
  let unsubbed = false;
  const rawUnsub = osOn("http:progress", (payload: unknown) => {
    if (unsubbed) return;
    if (!payload || typeof payload !== "object") return;
    const p = payload as Partial<HttpProgress> & { id?: unknown };
    if (p.id !== progressId) return;
    if (p.phase !== "download" && p.phase !== "upload") return;
    if (typeof p.loaded !== "number") return;
    if (p.total !== null && typeof p.total !== "number") return;
    onProgress({ phase: p.phase, loaded: p.loaded, total: p.total });
  });
  const unsub = () => {
    if (unsubbed) return;
    unsubbed = true;
    rawUnsub();
  };
  try {
    return await osCall<T>(
      "http",
      method,
      { ...baseArgs, progressId },
      osDeadline(baseArgs.timeoutMs),
    );
  } finally {
    unsub();
  }
}

type ResponseType = "text" | "json" | "binary";

/** Issue a `request` call with an explicit method and response type. */
function requestAs<T>(
  url: string,
  method: NonNullable<HttpRequestOptions["method"]>,
  responseType: ResponseType,
  opts?: HttpRequestOptions,
): Promise<HttpResponse<T>> {
  const { onProgress, ...rest } = opts ?? {};
  return run("request", { url, method, responseType, ...rest }, onProgress);
}

/** Replace the base64 body of a `responseType: "binary"` reply with its bytes. */
async function decodeBinaryBody(
  res: Promise<HttpResponse<string>>,
): Promise<HttpResponse<Uint8Array>> {
  const r = await res;
  return { ...r, body: base64ToBytes(r.body) };
}

/**
 * Build the overloaded `request` function. Object-literal methods
 * can't host multiple call signatures in TypeScript, so the overload
 * lives on a free function we then mount on `http.request`.
 */
interface HttpRequestFn {
  (
    url: string,
    opts: HttpRequestOptions & { responseType: "binary" },
  ): Promise<HttpResponse<Uint8Array>>;
  <T = string>(
    url: string,
    opts?: HttpRequestOptions & { responseType?: "text" | "json" },
  ): Promise<HttpResponse<T>>;
}
function makeRequest(): HttpRequestFn {
  return (async (
    url: string,
    opts?: HttpRequestOptions & { responseType?: ResponseType },
  ): Promise<HttpResponse<unknown>> => {
    const { onProgress, responseType, ...rest } = opts ?? {};
    if (responseType === "binary") {
      return decodeBinaryBody(
        run<HttpResponse<string>>("request", { url, responseType: "binary", ...rest }, onProgress),
      );
    }
    return run<HttpResponse<unknown>>(
      "request",
      { url, responseType: responseType ?? "text", ...rest },
      onProgress,
    );
  }) as HttpRequestFn;
}

/** Frontend bindings for the host's `http` API (request, JSON, binary, download). */
export const http = {
  /** Issue a GET; body decoded as text. */
  get<T = string>(
    url: string,
    opts?: Omit<HttpRequestOptions, "method" | "body">,
  ): Promise<HttpResponse<T>> {
    return requestAs(url, "GET", "text", opts);
  },
  /** Issue a GET; body parsed as JSON. */
  getJson<T = unknown>(
    url: string,
    opts?: Omit<HttpRequestOptions, "method" | "body">,
  ): Promise<HttpResponse<T>> {
    return requestAs(url, "GET", "json", opts);
  },
  /** Issue a GET; body returned as raw bytes. */
  getBinary(
    url: string,
    opts?: Omit<HttpRequestOptions, "method" | "body">,
  ): Promise<HttpResponse<Uint8Array>> {
    return decodeBinaryBody(requestAs(url, "GET", "binary", opts));
  },
  /** Issue a POST; body decoded as text by default. */
  post<T = string>(url: string, opts?: HttpRequestOptions): Promise<HttpResponse<T>> {
    return requestAs(url, "POST", "text", opts);
  },
  /**
   * Generic request with caller-supplied method. Overloaded so the
   * binary path returns `Uint8Array` while the text/json paths return
   * `string` / `T`. Callers MUST pick a `responseType` for binary
   * (`"binary"`) so the type system can guarantee they get bytes back
   * — silently changing the runtime shape based on the URL or method
   * is the previous foot-gun this overload guards against.
   * @example
   *   const r = await http.request<unknown>("https://api/x", { responseType: "json" });
   *   const buf = await http.request("https://api/blob", { responseType: "binary" });
   */
  request: makeRequest(),
  /** Stream-download a URL straight to disk; resolves with the file path and byte count, rejects on a non-2xx status. */
  download(
    url: string,
    dest: string,
    opts?: {
      headers?: Record<string, string>;
      timeoutMs?: number;
      onProgress?: (p: HttpProgress) => void;
    },
  ): Promise<{ path: string; bytes: number }> {
    const { onProgress, ...rest } = opts ?? {};
    return run("download", { url, dest, ...rest }, onProgress);
  },
};
