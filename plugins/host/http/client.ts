import { base64ToBytes, osCall, osDeadline, osOn } from "@tynd/core/client/bridge";

/** Generic HTTP response envelope returned by every `http.*` call. */
export interface HttpResponse<T = string> {
  status: number;
  statusText: string;
  headers: Record<string, string>;
  /** URL of the final response, after redirects. */
  url: string;
  /** Whether at least one redirect was followed. */
  redirected: boolean;
  body: T;
}

/** Response of `http.stream`: the head, plus the body as it arrives. */
export interface HttpStreamResponse extends Omit<HttpResponse, "body"> {
  /** Body chunks in arrival order. Cancelling the stream aborts the request. */
  body: ReadableStream<Uint8Array>;
}

/** Progress event emitted during an in-flight upload or download. */
export interface HttpProgress {
  phase: "upload" | "download";
  loaded: number;
  total: number | null;
}

/** How redirects (301, 302, 303, 307, 308) are handled. */
export type HttpRedirect = "follow" | "manual" | "error";

/** Transport options shared by every call. */
export interface HttpTransportOptions {
  headers?: Record<string, string>;
  /** Overall deadline in ms (default 60 s for requests; none for `download` / `stream`). */
  timeoutMs?: number;
  /** Deadline in ms for opening the connection (TCP + TLS). */
  connectTimeoutMs?: number;
  /**
   * `"follow"` (default) follows up to `maxRedirects` hops, checking every hop
   * against the security policy; `"manual"` returns the 3xx response;
   * `"error"` rejects on the first redirect.
   */
  redirect?: HttpRedirect;
  /** Most redirects followed before rejecting (default 10, max 50). */
  maxRedirects?: number;
  /**
   * Proxy URL (`http://`, `https://`, `socks5://`, `socks5h://`, `socks4://`,
   * `socks4a://`, credentials allowed), or `false` to bypass the
   * `HTTPS_PROXY` / `HTTP_PROXY` / `ALL_PROXY` environment proxy.
   */
  proxy?: string | false;
  /**
   * Send and store cookies: `true` uses the `"default"` jar, a string names a
   * jar. Jars live in memory for the app's lifetime and belong to the caller:
   * the backend, and each webview per origin, have their own jars (the same
   * name in two of them is two jars).
   */
  cookies?: boolean | string;
  /** Abort the call; it rejects with an `AbortError`. */
  signal?: AbortSignal;
}

/** Per-call options for `http.request` and friends. */
export interface HttpRequestOptions extends HttpTransportOptions {
  method?: "GET" | "POST" | "PUT" | "PATCH" | "DELETE" | "HEAD" | "OPTIONS";
  body?: string | Record<string, unknown> | unknown[];
  onProgress?: (p: HttpProgress) => void;
}

/** Options of `http.stream`. */
export interface HttpStreamOptions extends Omit<HttpRequestOptions, "onProgress"> {
  /**
   * Reject once no body byte arrived for this long, in ms (default 300 000 =
   * 5 min, max 24 h). Send heartbeats on long-idle server-sent-event streams.
   */
  idleTimeoutMs?: number;
}

/** Options of `http.download`. */
export interface HttpDownloadOptions extends HttpTransportOptions {
  onProgress?: (p: HttpProgress) => void;
}

function newId(prefix: string): string {
  return `${prefix}_${Date.now().toString(36)}_${Math.random().toString(36).slice(2, 10)}`;
}

function abortError(signal: AbortSignal): unknown {
  return signal.reason ?? new DOMException("The operation was aborted.", "AbortError");
}

/** Wire args: the options minus the client-only fields. */
function wireArgs(
  base: Record<string, unknown>,
  opts: HttpTransportOptions & { onProgress?: unknown },
): Record<string, unknown> {
  const { signal: _signal, onProgress: _onProgress, ...rest } = opts;
  return { ...base, ...rest };
}

/**
 * Run `call` with `signal` wired to `http.abort`: an abort rejects at once
 * and tells the host to stop at its next chunk boundary.
 */
async function withAbort<T>(
  args: Record<string, unknown>,
  signal: AbortSignal | undefined,
  call: (args: Record<string, unknown>) => Promise<T>,
): Promise<T> {
  if (!signal) return call(args);
  if (signal.aborted) throw abortError(signal);
  const abortId = newId("abort");
  let onAbort = () => undefined;
  const aborted = new Promise<never>((_, reject) => {
    onAbort = () => {
      void osCall("http", "abort", { id: abortId }).catch(() => undefined);
      reject(abortError(signal));
    };
    signal.addEventListener("abort", onAbort, { once: true });
  });
  try {
    return await Promise.race([call({ ...args, abortId }), aborted]);
  } finally {
    signal.removeEventListener("abort", onAbort);
  }
}

async function run<T>(
  method: "request" | "download",
  baseArgs: Record<string, unknown>,
  signal: AbortSignal | undefined,
  onProgress?: (p: HttpProgress) => void,
): Promise<T> {
  const deadline = osDeadline(baseArgs.timeoutMs);
  if (!onProgress) {
    return withAbort(baseArgs, signal, (args) => osCall<T>("http", method, args, deadline));
  }
  const progressId = newId("http");
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
    return await withAbort({ ...baseArgs, progressId }, signal, (args) =>
      osCall<T>("http", method, args, deadline),
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
  const o = opts ?? {};
  return run("request", wireArgs({ url, method, responseType }, o), o.signal, o.onProgress);
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
    const { responseType, ...o } = opts ?? {};
    if (responseType === "binary") {
      return decodeBinaryBody(
        run<HttpResponse<string>>(
          "request",
          wireArgs({ url, responseType: "binary" }, o),
          o.signal,
          o.onProgress,
        ),
      );
    }
    return run<HttpResponse<unknown>>(
      "request",
      wireArgs({ url, responseType: responseType ?? "text" }, o),
      o.signal,
      o.onProgress,
    );
  }) as HttpRequestFn;
}

interface StreamEvent {
  id?: unknown;
  kind?: unknown;
  data?: unknown;
  status?: number;
  statusText?: string;
  headers?: Record<string, string>;
  url?: string;
  redirected?: boolean;
}

/** What a `StreamSession` reports to and listens for. */
interface StreamTarget {
  url: string;
  streamId: string;
  abortId: string;
  signal: AbortSignal | undefined;
  resolve: (res: HttpStreamResponse) => void;
  reject: (error: unknown) => void;
}

/** Lifecycle of one `http.stream` call, fed by its `http:stream` events. */
class StreamSession {
  private controller: ReadableStreamDefaultController<Uint8Array> | undefined;
  private finished = false;
  private headSeen = false;
  private off: () => void = () => undefined;
  private onAbort: () => void = () => undefined;
  readonly body: ReadableStream<Uint8Array>;
  private readonly target: StreamTarget;

  constructor(target: StreamTarget) {
    this.target = target;
    this.body = new ReadableStream<Uint8Array>({
      start: (c) => {
        this.controller = c;
      },
      cancel: () => {
        if (this.finish()) this.abortHost();
      },
    });
  }

  /** Subscribe to the stream events and to `signal`. */
  listen(): void {
    this.off = osOn("http:stream", (raw: unknown) => this.onEvent(raw));
    const { signal } = this.target;
    if (!signal) return;
    this.onAbort = () => {
      this.abortHost();
      this.fail(abortError(signal));
    };
    signal.addEventListener("abort", this.onAbort, { once: true });
  }

  /** Reject before the head, error the body after it. */
  fail(error: unknown): void {
    if (!this.finish()) return;
    if (this.headSeen) this.controller?.error(error);
    else this.target.reject(error);
  }

  /** Mark the session done; `false` if it already was. */
  private finish(): boolean {
    if (this.finished) return false;
    this.finished = true;
    this.off();
    this.target.signal?.removeEventListener("abort", this.onAbort);
    return true;
  }

  private abortHost(): void {
    void osCall("http", "abort", { id: this.target.abortId }).catch(() => undefined);
  }

  private onEvent(raw: unknown): void {
    if (this.finished || !raw || typeof raw !== "object") return;
    const event = raw as StreamEvent;
    if (event.id !== this.target.streamId) return;
    if (event.kind === "head") {
      this.headSeen = true;
      this.target.resolve({
        status: event.status ?? 0,
        statusText: event.statusText ?? "",
        headers: event.headers ?? {},
        url: event.url ?? this.target.url,
        redirected: event.redirected ?? false,
        body: this.body,
      });
    } else if (event.kind === "chunk" && typeof event.data === "string") {
      this.controller?.enqueue(base64ToBytes(event.data));
    } else if (event.kind === "end" && this.finish()) {
      this.controller?.close();
    }
  }
}

/** `http.stream` implementation (see the method docs). */
function openStream(url: string, opts: HttpStreamOptions = {}): Promise<HttpStreamResponse> {
  const { signal } = opts;
  if (signal?.aborted) return Promise.reject(abortError(signal));
  const streamId = newId("stream");
  const abortId = newId("abort");
  const args = wireArgs({ url, streamId, abortId }, opts);
  return new Promise<HttpStreamResponse>((resolve, reject) => {
    const session = new StreamSession({ url, streamId, abortId, signal, resolve, reject });
    session.listen();
    osCall("http", "stream", args, osDeadline(opts.timeoutMs)).then(undefined, (error) =>
      session.fail(error),
    );
  });
}

/** Frontend bindings for the host's `http` API (request, JSON, binary, stream, download). */
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
  /**
   * Issue a request and read the body as it arrives. Resolves once the
   * response head is in; `body` yields the chunks. No overall deadline
   * unless `timeoutMs` is set, so it suits server-sent events; it rejects
   * after `idleTimeoutMs` (default 5 min) without data, and the host stops it
   * when the calling page navigates, reloads or closes.
   * @example
   *   const res = await http.stream("https://api.example.com/events");
   *   for await (const chunk of res.body) console.log(chunk.byteLength);
   */
  stream: openStream,
  /** Stream-download a URL straight to disk; resolves with the file path and byte count, rejects on a non-2xx status. */
  download(
    url: string,
    dest: string,
    opts?: HttpDownloadOptions,
  ): Promise<{ path: string; bytes: number }> {
    const o = opts ?? {};
    return run("download", wireArgs({ url, dest }, o), o.signal, o.onProgress);
  },
  /** Empty the caller's cookie jar named `jar`, or all of the caller's jars when omitted. */
  clearCookies(jar?: string): Promise<void> {
    return osCall("http", "clearCookies", jar === undefined ? {} : { jar });
  },
};
