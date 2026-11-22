import {
  decodeMessage,
  encodeRequest,
  FrameDecoder,
  RpcError,
  type RpcMessage,
} from "./protocol.ts";

/** Minimal duplex byte stream the client runs on; `node:net` sockets adapt to it, tests mock it. */
export interface Transport {
  write(data: Uint8Array): void;
  onData(listener: (chunk: Uint8Array) => void): void;
  /** Called once when the stream ends or fails. */
  onClose(listener: (error?: Error) => void): void;
  close(): void;
}

export interface WindowInfo {
  label: string;
  title: string;
  visible: boolean;
  focused: boolean;
  size: { width: number; height: number };
  position: { x: number; y: number };
  [key: string]: unknown;
}

export interface ElementRef {
  ref: string;
  x: number;
  y: number;
  w: number;
  h: number;
}

/** What the debug helper accepts to locate an element: a `ref` from a snapshot or find, or viewport coordinates. */
export type Target = { ref: string } | { x: number; y: number };

export interface SnapshotNode {
  ref: string;
  tag: string;
  text: string;
  x: number;
  y: number;
  w: number;
  h: number;
}

export type SnapshotMode = "map" | "html" | "state";

export interface MapSnapshot {
  map: SnapshotNode[];
  viewport: { w: number; h: number };
}

export interface HtmlSnapshot {
  html: string;
}

export interface StateSnapshot {
  url: string;
  title: string;
  scrollX: number;
  scrollY: number;
  viewport: { w: number; h: number };
  readyState: string;
}

export interface IpcFrame {
  id: string;
  ts: number;
  direction: "request" | "response" | "event";
  channel: string;
  payload: unknown;
  fn?: string;
  error?: { code?: string; message?: string };
  duration_ms?: number;
}

export interface ScreenshotResult {
  width: number;
  height: number;
  /** Decoded PNG bytes. */
  png: Uint8Array;
}

export interface FillField {
  /** Element `ref`; provide `selector` instead to resolve one with `document.querySelector`. */
  ref?: string;
  selector?: string;
  value: string;
}

export interface WindowOptions {
  /** Window label, `"main"` by default. */
  label?: string;
}

export interface CallOptions {
  /** Request timeout in milliseconds. */
  timeout?: number;
}

export interface DebugClientOptions {
  /** Default per-request timeout in milliseconds (30 s). */
  requestTimeout?: number;
}

/** Thrown when a DOM helper reports `{ ok: false }` or a selector matches nothing. */
export class ActionError extends Error {
  readonly action: string;

  constructor(action: string, message: string) {
    super(`${action} failed: ${message}`);
    this.name = "ActionError";
    this.action = action;
  }
}

interface Pending {
  method: string;
  resolve: (value: unknown) => void;
  reject: (error: Error) => void;
  timer: ReturnType<typeof setTimeout>;
}

const DEFAULT_REQUEST_TIMEOUT_MS = 30_000;

/** Typed JSON-RPC client for the Tynd debug protocol. */
export class DebugClient {
  readonly #transport: Transport;
  readonly #decoder = new FrameDecoder();
  readonly #pending = new Map<string, Pending>();
  readonly #notifications = new Set<(method: string, params: unknown) => void>();
  readonly #requestTimeout: number;
  #nextId = 1;
  #closed: Error | undefined;

  constructor(transport: Transport, options: DebugClientOptions = {}) {
    this.#transport = transport;
    this.#requestTimeout = options.requestTimeout ?? DEFAULT_REQUEST_TIMEOUT_MS;
    transport.onData((chunk) => this.#receive(chunk));
    transport.onClose((error) => this.#fail(error ?? new Error("debug connection closed")));
  }

  get closed(): boolean {
    return this.#closed !== undefined;
  }

  /** Subscribe to server notifications (subscription frames). Returns an unsubscribe function. */
  onNotification(listener: (method: string, params: unknown) => void): () => void {
    this.#notifications.add(listener);
    return () => this.#notifications.delete(listener);
  }

  /** Authenticate with the token published in the discovery file. Must be the first call. */
  async hello(token: string): Promise<void> {
    const reply = await this.call<{ cookie?: string }>("bridge.hello", { cookie: token });
    if (reply?.cookie !== token) {
      throw new Error("debug handshake did not echo the token; wrong process?");
    }
  }

  /** Send a request and resolve with its `result`; rejects with {@link RpcError} on a JSON-RPC error. */
  call<T = unknown>(method: string, params: unknown = {}, options: CallOptions = {}): Promise<T> {
    if (this.#closed) return Promise.reject(this.#closed);
    const id = String(this.#nextId++);
    const timeout = options.timeout ?? this.#requestTimeout;
    return new Promise<T>((resolve, reject) => {
      const timer = setTimeout(() => {
        this.#pending.delete(id);
        reject(new Error(`debug request ${method} timed out after ${timeout} ms`));
      }, timeout);
      this.#pending.set(id, {
        method,
        resolve: resolve as (value: unknown) => void,
        reject,
        timer,
      });
      try {
        this.#transport.write(encodeRequest(id, method, params));
      } catch (error) {
        clearTimeout(timer);
        this.#pending.delete(id);
        reject(error instanceof Error ? error : new Error(String(error)));
      }
    });
  }

  /** Close the connection; pending requests reject. */
  close(): void {
    if (!this.#closed) {
      this.#fail(new Error("debug client closed"));
      this.#transport.close();
    }
  }

  // --- typed methods -------------------------------------------------------------------------

  /** `app.info` */
  appInfo(): Promise<Record<string, unknown>> {
    return this.call("app.info");
  }

  /** `app.quit` */
  async quit(): Promise<void> {
    await this.call("app.quit");
  }

  /** `window.list` */
  windowList(): Promise<WindowInfo[]> {
    return this.call("window.list");
  }

  /**
   * `webview.eval`: run `script` in the page. The host returns the JSON text of the value; it is
   * parsed when possible. Pass `raw: true` to get the untouched result.
   */
  async eval<T = unknown>(
    script: string,
    options: WindowOptions & { raw?: boolean } = {},
  ): Promise<T> {
    const result = await this.call("webview.eval", withLabel({ script }, options));
    if (options.raw || typeof result !== "string") return result as T;
    try {
      return JSON.parse(result) as T;
    } catch {
      return result as T;
    }
  }

  /** `webview.find_element`: resolve a CSS selector to an element ref, or `null` when absent. */
  async findElement(selector: string, options: WindowOptions = {}): Promise<ElementRef | null> {
    const found = await this.call<(ElementRef & { error?: string }) | null>(
      "webview.find_element",
      withLabel({ selector }, options),
    );
    if (found?.error) throw new ActionError("findElement", found.error);
    return found;
  }

  /** `webview.click`. `target` is a CSS selector, a `{ ref }` or viewport coordinates `{ x, y }`. */
  async click(target: string | Target, options: WindowOptions = {}): Promise<void> {
    const resolved = await this.#target(target, options);
    expectOk("click", await this.call("webview.click", withLabel({ target: resolved }, options)));
  }

  /** `webview.type`: append `text` to the target (or the focused element when omitted). */
  async type(text: string, target?: string | Target, options: WindowOptions = {}): Promise<void> {
    const params: { text: string; target?: Target } = { text };
    if (target !== undefined) params.target = await this.#target(target, options);
    expectOk("type", await this.call("webview.type", withLabel(params, options)));
  }

  /** `webview.fill`: replace the value of one or more fields. */
  async fill(fields: FillField[], options: WindowOptions = {}): Promise<void> {
    const resolved: { ref: string; value: string }[] = [];
    for (const field of fields) {
      const ref = field.ref ?? (await this.#ref(field.selector, options));
      resolved.push({ ref, value: field.value });
    }
    expectOk("fill", await this.call("webview.fill", withLabel({ fields: resolved }, options)));
  }

  /** `webview.press_key`: dispatch a key combination such as `"Enter"` or `"ctrl+k"`. */
  async pressKey(key: string, options: WindowOptions = {}): Promise<void> {
    expectOk("pressKey", await this.call("webview.press_key", withLabel({ key }, options)));
  }

  /** `webview.snapshot` */
  snapshot(options?: WindowOptions & { mode?: "map" }): Promise<MapSnapshot>;
  snapshot(options: WindowOptions & { mode: "html" }): Promise<HtmlSnapshot>;
  snapshot(options: WindowOptions & { mode: "state" }): Promise<StateSnapshot>;
  snapshot(options: WindowOptions & { mode?: SnapshotMode } = {}): Promise<unknown> {
    return this.call("webview.snapshot", withLabel({ mode: options.mode ?? "map" }, options));
  }

  /** `webview.console_logs` */
  consoleLogs(options: WindowOptions & { clear?: boolean } = {}): Promise<unknown[]> {
    return this.call("webview.console_logs", withLabel({ clear: options.clear ?? false }, options));
  }

  /** `screenshot.window`: capture a window as PNG. */
  async screenshot(options: WindowOptions = {}): Promise<ScreenshotResult> {
    const reply = await this.call<{ width: number; height: number; png_base64: string }>(
      "screenshot.window",
      withLabel({}, options),
    );
    return {
      width: reply.width,
      height: reply.height,
      png: new Uint8Array(Buffer.from(reply.png_base64, "base64")),
    };
  }

  /** `ipc.recent`: the last `n` IPC frames seen by the host. */
  ipcRecent(n = 64): Promise<IpcFrame[]> {
    return this.call("ipc.recent", { n });
  }

  // --- internals -----------------------------------------------------------------------------

  async #ref(selector: string | undefined, options: WindowOptions): Promise<string> {
    if (selector === undefined)
      throw new ActionError("fill", "each field needs a ref or a selector");
    const target = await this.#target(selector, options);
    if ("ref" in target) return target.ref;
    throw new ActionError("fill", "unreachable target");
  }

  async #target(target: string | Target, options: WindowOptions): Promise<Target> {
    if (typeof target !== "string") return target;
    const found = await this.findElement(target, options);
    if (!found) throw new ActionError("find", `no element matches ${JSON.stringify(target)}`);
    return { ref: found.ref };
  }

  #receive(chunk: Uint8Array): void {
    let frames: Uint8Array[];
    try {
      frames = this.#decoder.push(chunk);
    } catch (error) {
      this.#fail(error instanceof Error ? error : new Error(String(error)));
      this.#transport.close();
      return;
    }
    for (const frame of frames) {
      let message: RpcMessage;
      try {
        message = decodeMessage(frame);
      } catch {
        continue;
      }
      this.#dispatch(message);
    }
  }

  #dispatch(message: RpcMessage): void {
    const id = message.id;
    if (id === undefined || id === null) {
      if (message.error) {
        // Unsolicited error frame, e.g. `already_attached` when another client holds the socket.
        this.#fail(new RpcError(message.error));
      } else if (message.method) {
        for (const listener of this.#notifications) listener(message.method, message.params);
      }
      return;
    }
    const pending = this.#pending.get(String(id));
    if (!pending) return;
    this.#pending.delete(String(id));
    clearTimeout(pending.timer);
    if (message.error) pending.reject(new RpcError(message.error, pending.method));
    else pending.resolve(message.result ?? null);
  }

  #fail(error: Error): void {
    if (this.#closed) return;
    this.#closed = error;
    for (const [id, pending] of this.#pending) {
      clearTimeout(pending.timer);
      pending.reject(error);
      this.#pending.delete(id);
    }
  }
}

function withLabel<T extends Record<string, unknown>>(
  params: T,
  options: WindowOptions,
): T & { label?: string } {
  return options.label === undefined ? params : { ...params, label: options.label };
}

function expectOk(action: string, result: unknown): void {
  const body = result as { ok?: boolean; error?: string } | null;
  if (body && body.ok === false) throw new ActionError(action, body.error ?? "unknown error");
}
