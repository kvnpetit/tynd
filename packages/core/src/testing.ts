import type { CallHandle, OsCallOptions } from "./client/_internal";
import { encodeIpcError } from "./errors";

export interface MockBackendCall {
  kind: "backend";
  fn: string;
  args: unknown[];
}

export interface MockOsCall {
  kind: "os";
  api: string;
  method: string;
  args: unknown;
}

export interface MockBinaryCall {
  kind: "binary";
  request: Request;
}

export type TyndMockCall = MockBackendCall | MockOsCall | MockBinaryCall;
export type MockBackendHandler = (...args: unknown[]) => unknown;
export type MockOsHandler = (args: unknown) => unknown;
/** Answers any OS call that has no `"api.method"` handler. */
export type MockOsFallback = (api: string, method: string, args: unknown) => unknown;

export interface InstallTyndMockOptions {
  backend?: Record<string, MockBackendHandler>;
  /** Keys use `"api.method"`, for example `"dialog.openFile"`. */
  os?: Record<string, MockOsHandler>;
  /**
   * Answers OS calls that match no `os` handler, for example `() => null` to
   * accept every call and only assert on `calls`. Without it, an unhandled OS
   * call rejects.
   */
  osFallback?: MockOsFallback;
  /**
   * Handles `tynd-bin://` requests. Return a Response, bytes, text, or
   * `undefined` for an empty 200 response.
   */
  binary?: (
    request: Request,
  ) =>
    | Response
    | Uint8Array
    | string
    | undefined
    | Promise<Response | Uint8Array | string | undefined>;
  windowLabel?: string;
}

export interface TyndMock {
  readonly calls: TyndMockCall[];
  emit(name: string, payload?: unknown): void;
  emitOs(name: string, payload?: unknown): void;
  setBackend(name: string, handler: MockBackendHandler): void;
  setOs(api: string, method: string, handler: MockOsHandler): void;
  clear(): void;
  restore(): void;
}

type Handler = (payload: unknown) => void;

function isAsyncIterable(value: unknown): value is AsyncIterable<unknown> {
  return (
    value !== null &&
    (typeof value === "object" || typeof value === "function") &&
    typeof (value as { [Symbol.asyncIterator]?: unknown })[Symbol.asyncIterator] === "function"
  );
}

interface MockWaiter {
  resolve(result: IteratorResult<unknown>): void;
  reject(reason: unknown): void;
}

class MockCallState {
  private readonly buffered: unknown[] = [];
  private readonly waiters: MockWaiter[] = [];
  private iterator: AsyncIterator<unknown> | null = null;
  private done = false;
  private cancelled = false;
  private error: unknown;
  private hasError = false;
  private rejectCancel!: (reason: unknown) => void;
  /** Rejects when the call is cancelled, mirroring the real shim settling its final promise. */
  readonly cancelSignal = new Promise<never>((_, reject) => {
    this.rejectCancel = reject;
  });

  private readonly fn: string;

  constructor(fn: string) {
    this.fn = fn;
    this.cancelSignal.catch(() => undefined);
  }

  async consume(value: unknown): Promise<unknown> {
    if (!isAsyncIterable(value)) {
      this.finish();
      return value;
    }
    this.iterator = value[Symbol.asyncIterator]();
    if (this.cancelled) {
      this.closeIterator();
      return;
    }
    while (!this.cancelled) {
      const result = await this.iterator.next();
      if (this.cancelled) return;
      if (result.done) {
        this.finish();
        return result.value;
      }
      this.push(result.value);
    }
    return;
  }

  fail(reason: unknown): never {
    this.done = true;
    this.error = reason;
    this.hasError = true;
    for (const waiter of this.waiters.splice(0)) waiter.reject(reason);
    throw reason;
  }

  /**
   * Same observable behaviour as the real shim: the final promise and every
   * parked `next()` reject with a CANCELLED `TyndIpcError` envelope, buffered
   * chunks are dropped, and the backend iterator is closed best-effort.
   * A no-op once the call has settled or was already cancelled.
   */
  async cancel(): Promise<IteratorResult<unknown>> {
    if (this.done || this.cancelled) return { value: undefined, done: true };
    this.cancelled = true;
    this.done = true;
    this.buffered.length = 0;
    const err = Object.assign(
      new Error(encodeIpcError({ code: "CANCELLED", message: "call cancelled", fn: this.fn })),
      { code: "CANCELLED" },
    );
    this.error = err;
    this.hasError = true;
    for (const waiter of this.waiters.splice(0)) waiter.reject(err);
    this.rejectCancel(err);
    this.closeIterator();
    return { value: undefined, done: true };
  }

  createIterator(): AsyncIterator<unknown> {
    return {
      next: () => this.next(),
      return: () => this.cancel(),
    };
  }

  private closeIterator(): void {
    // Not awaited: a generator parked in `next()` would only return after it resumes.
    Promise.resolve(this.iterator?.return?.()).catch(() => undefined);
  }

  private next(): Promise<IteratorResult<unknown>> {
    if (this.buffered.length > 0) {
      return Promise.resolve({ value: this.buffered.shift(), done: false });
    }
    if (this.done) {
      return this.hasError
        ? Promise.reject(this.error)
        : Promise.resolve({ value: undefined, done: true });
    }
    return new Promise((resolve, reject) => this.waiters.push({ resolve, reject }));
  }

  private finish(): void {
    this.done = true;
    for (const waiter of this.waiters.splice(0)) {
      waiter.resolve({ value: undefined, done: true });
    }
  }

  private push(value: unknown): void {
    const waiter = this.waiters.shift();
    if (waiter) waiter.resolve({ value, done: false });
    else this.buffered.push(value);
  }
}

function mockCallHandle(fn: string, execute: () => unknown): CallHandle {
  const state = new MockCallState(fn);
  const execution = Promise.resolve()
    .then(execute)
    .then((value) => state.consume(value))
    .catch((reason) => state.fail(reason));
  const final: Promise<unknown> = Promise.race([execution, state.cancelSignal]);

  return Object.assign(final, {
    cancel: () => {
      final.catch(() => undefined);
      return state.cancel();
    },
    [Symbol.asyncIterator]: () => state.createIterator(),
  }) as unknown as CallHandle;
}
function responseFromMock(value: Response | Uint8Array | string | undefined): Response {
  if (value instanceof Response) return value;
  return new Response((value ?? null) as BodyInit | null, { status: 200 });
}

class MockEventHub {
  private readonly handlers = new Map<string, Set<Handler>>();

  subscribe(name: string, handler: Handler): () => void {
    const set = this.handlers.get(name) ?? new Set<Handler>();
    set.add(handler);
    this.handlers.set(name, set);
    return () => {
      set.delete(handler);
      if (set.size === 0) this.handlers.delete(name);
    };
  }

  emit(name: string, payload: unknown): void {
    for (const handler of [...(this.handlers.get(name) ?? [])]) handler(payload);
  }

  off(name: string, handler: Handler): void {
    this.handlers.get(name)?.delete(handler);
  }
}

interface MockState {
  backend: Map<string, MockBackendHandler>;
  os: Map<string, MockOsHandler>;
  osFallback: MockOsFallback | undefined;
  backendEvents: MockEventHub;
  osEvents: MockEventHub;
  calls: TyndMockCall[];
}

interface GlobalSnapshot {
  globalRecord: { window?: Window };
  hadWindow: boolean;
  previousWindow: Window | undefined;
  targetWindow: Window;
  previousBridge: Window["__tynd__"] | undefined;
  previousLabel: string | undefined;
  originalFetch: typeof fetch;
}

function captureGlobals(): GlobalSnapshot {
  const globalRecord = globalThis as unknown as { window?: Window };
  const previousWindow = globalRecord.window;
  const targetWindow = previousWindow ?? ({} as Window);
  return {
    globalRecord,
    hadWindow: "window" in globalRecord,
    previousWindow,
    targetWindow,
    previousBridge: targetWindow.__tynd__,
    previousLabel: targetWindow.__TYND_WINDOW_LABEL__,
    originalFetch: globalThis.fetch,
  };
}

/** Like the real shim: no deadline unless the caller passes `timeoutMs`. */
function withOsDeadline(
  api: string,
  method: string,
  opts: OsCallOptions | undefined,
  run: () => Promise<unknown>,
): Promise<unknown> {
  const timeoutMs = opts?.timeoutMs;
  if (typeof timeoutMs !== "number" || timeoutMs <= 0) return run();
  return new Promise((resolve, reject) => {
    const timer = setTimeout(
      () => reject(new Error(`os_call(${api}.${method}) timed out after ${timeoutMs}ms`)),
      timeoutMs,
    );
    run().then(
      (value) => {
        clearTimeout(timer);
        resolve(value);
      },
      (reason) => {
        clearTimeout(timer);
        reject(reason);
      },
    );
  });
}

function createBridge(state: MockState): Window["__tynd__"] {
  return {
    call(fn, args) {
      state.calls.push({ kind: "backend", fn, args });
      return mockCallHandle(fn, () => {
        const handler = state.backend.get(fn);
        if (!handler) throw new Error(`[tynd mock] no backend handler for "${fn}"`);
        return handler(...args);
      });
    },
    os_call(api, method, args, opts) {
      state.calls.push({ kind: "os", api, method, args });
      const handler = state.os.get(`${api}.${method}`);
      const fallback = state.osFallback;
      if (handler) return withOsDeadline(api, method, opts, async () => handler(args));
      if (fallback)
        return withOsDeadline(api, method, opts, async () => fallback(api, method, args));
      return Promise.reject(new Error(`[tynd mock] no OS handler for "${api}.${method}"`));
    },
    os_on: (name, handler) => state.osEvents.subscribe(name, handler),
    on: (name, handler) => state.backendEvents.subscribe(name, handler),
    off: (name, handler) => state.backendEvents.off(name, handler),
  };
}

/** `tynd-bin://` origin, plus the `http://tynd-bin.localhost` form used on Windows and Android. */
const BIN_URL_PREFIXES = ["tynd-bin://", "http://tynd-bin.localhost/"];

function installBinaryFetch(
  handler: NonNullable<InstallTyndMockOptions["binary"]>,
  state: MockState,
  originalFetch: typeof fetch,
): void {
  globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
    const request = new Request(input, init);
    if (!BIN_URL_PREFIXES.some((prefix) => request.url.startsWith(prefix))) {
      return originalFetch(input, init);
    }
    state.calls.push({ kind: "binary", request });
    return responseFromMock(await handler(request));
  }) as typeof fetch;
}

function createRestore(snapshot: GlobalSnapshot): () => void {
  let restored = false;
  return () => {
    if (restored) return;
    restored = true;
    globalThis.fetch = snapshot.originalFetch;
    if (snapshot.previousBridge) snapshot.targetWindow.__tynd__ = snapshot.previousBridge;
    else Reflect.deleteProperty(snapshot.targetWindow, "__tynd__");
    if (snapshot.previousLabel === undefined) {
      Reflect.deleteProperty(snapshot.targetWindow, "__TYND_WINDOW_LABEL__");
    } else {
      snapshot.targetWindow.__TYND_WINDOW_LABEL__ = snapshot.previousLabel;
    }
    if (snapshot.hadWindow) snapshot.globalRecord.window = snapshot.previousWindow;
    else Reflect.deleteProperty(snapshot.globalRecord, "window");
  };
}

function createMockController(state: MockState, restore: () => void): TyndMock {
  return {
    calls: state.calls,
    emit: (name, payload) => state.backendEvents.emit(name, payload),
    emitOs: (name, payload) => state.osEvents.emit(name, payload),
    setBackend: (name, handler) => state.backend.set(name, handler),
    setOs: (api, method, handler) => state.os.set(`${api}.${method}`, handler),
    clear: () => {
      state.calls.length = 0;
    },
    restore,
  };
}

/** Install an in-memory Tynd bridge for unit/component tests. */
export function installTyndMock(options: InstallTyndMockOptions = {}): TyndMock {
  const snapshot = captureGlobals();
  const state: MockState = {
    backend: new Map(Object.entries(options.backend ?? {})),
    os: new Map(Object.entries(options.os ?? {})),
    osFallback: options.osFallback,
    backendEvents: new MockEventHub(),
    osEvents: new MockEventHub(),
    calls: [],
  };

  snapshot.targetWindow.__TYND_WINDOW_LABEL__ = options.windowLabel ?? "main";
  snapshot.targetWindow.__tynd__ = createBridge(state);
  snapshot.globalRecord.window = snapshot.targetWindow;
  if (options.binary) installBinaryFetch(options.binary, state, snapshot.originalFetch);

  return createMockController(state, createRestore(snapshot));
}
