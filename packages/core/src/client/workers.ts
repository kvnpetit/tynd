import { tynd } from "../logger";
import {
  createWorkerTaskScript,
  MAX_PENDING_WORKER_RUNS,
  MAX_WORKERS,
  parseWorkerResponse,
} from "../worker-protocol";

/** Handle to a sandboxed Web Worker owned by the renderer. */
export interface WorkerHandle {
  id: number;
  run<Out = unknown, In = unknown>(input: In): Promise<Out>;
  terminate(): Promise<void>;
}

let workerSequence = 0;
const workerIds = new Set<number>();

function allocateWorkerId(): number {
  for (;;) {
    workerSequence = workerSequence >= Number.MAX_SAFE_INTEGER ? 1 : workerSequence + 1;
    if (!workerIds.has(workerSequence)) return workerSequence;
  }
}

/**
 * Identifiers that always resolve in a fresh Worker context — both built-in
 * globals and common host-injected ones. Used by the closure-detection
 * heuristic below so we don't warn about every `console.log`.
 */
const KNOWN_WORKER_GLOBALS = new Set([
  "console",
  "globalThis",
  "self",
  "Promise",
  "Array",
  "Object",
  "Math",
  "JSON",
  "fetch",
  "URL",
  "URLSearchParams",
  "Date",
  "RegExp",
  "Symbol",
  "Error",
  "TypeError",
  "RangeError",
  "SyntaxError",
  "Number",
  "String",
  "Boolean",
  "Map",
  "Set",
  "WeakMap",
  "WeakSet",
  "ArrayBuffer",
  "Uint8Array",
  "Uint16Array",
  "Uint32Array",
  "Int8Array",
  "Int16Array",
  "Int32Array",
  "Float32Array",
  "Float64Array",
  "DataView",
  "TextEncoder",
  "TextDecoder",
  "atob",
  "btoa",
  "postMessage",
  "addEventListener",
  "removeEventListener",
  "setTimeout",
  "setInterval",
  "clearTimeout",
  "clearInterval",
  "crypto",
  "performance",
  "queueMicrotask",
  "structuredClone",
]);

const JS_KEYWORDS = new Set([
  "return",
  "if",
  "else",
  "for",
  "while",
  "do",
  "const",
  "let",
  "var",
  "function",
  "async",
  "await",
  "true",
  "false",
  "null",
  "undefined",
  "throw",
  "try",
  "catch",
  "finally",
  "new",
  "typeof",
  "instanceof",
  "in",
  "of",
  "delete",
  "void",
  "yield",
  "switch",
  "case",
  "break",
  "continue",
  "default",
  "this",
  "super",
  "class",
  "extends",
  "static",
  "import",
  "export",
  "from",
  "as",
  "with",
]);

/**
 * Best-effort closure detection. Scans the function source for identifiers
 * that aren't arg names, locals declared with `const`/`let`/`var`, JS
 * keywords, or worker globals — and warns if any remain. False-positives
 * happen (e.g. minified code, rest params) so this is dev-mode only and
 * non-fatal: the message points at `args` which is the supported way to
 * pass values into the worker.
 */
function detectClosureRefs(fnStr: string): Set<string> {
  const idents = fnStr.match(/\b[a-zA-Z_$][a-zA-Z0-9_$]*\b/g) ?? [];
  const refs = new Set(idents);

  // Strip arg names from `function (a, b)` / `(a, b) => ...` / `function name(a, b)`.
  const argMatch = fnStr.match(
    /^\s*(?:async\s+)?(?:function\s*\*?\s*[a-zA-Z_$][a-zA-Z0-9_$]*?\s*)?\(([^)]*)\)/,
  );
  const argList = argMatch?.[1] ?? "";
  for (const piece of argList.split(",")) {
    const name = piece.trim().split(/[=\s:]/)[0];
    if (name) refs.delete(name);
  }
  // Single-arg arrow `x => ...` (no parens).
  const arrowMatch = fnStr.match(/^\s*(?:async\s+)?([a-zA-Z_$][a-zA-Z0-9_$]*)\s*=>/);
  if (arrowMatch?.[1]) refs.delete(arrowMatch[1]);

  // Strip locals declared inside the function.
  const declMatches = fnStr.matchAll(/\b(?:const|let|var)\s+([a-zA-Z_$][a-zA-Z0-9_$]*)/g);
  for (const m of declMatches) {
    if (m[1]) refs.delete(m[1]);
  }
  // Strip property names after `.` so `obj.foo` doesn't flag `foo`.
  const propMatches = fnStr.matchAll(/\.\s*([a-zA-Z_$][a-zA-Z0-9_$]*)/g);
  for (const m of propMatches) {
    if (m[1]) refs.delete(m[1]);
  }

  for (const k of JS_KEYWORDS) refs.delete(k);
  for (const g of KNOWN_WORKER_GLOBALS) refs.delete(g);
  return refs;
}

/**
 * Validate `workers.spawn` input. Throws synchronously for unrecoverable
 * cases (non-function, native [native code]) and warns in dev mode for
 * suspected closure variables. Returns the function source string so the
 * caller doesn't double-stringify.
 */
function validateTaskFn(taskFn: unknown): string {
  if (typeof taskFn !== "function") {
    throw new TypeError(
      `workers.spawn: expected a function, got ${taskFn === null ? "null" : typeof taskFn}`,
    );
  }
  const fnStr = (taskFn as () => unknown).toString();
  if (fnStr.includes("[native code]")) {
    throw new Error(
      "workers.spawn: native functions cannot be serialized to a Worker (use a wrapper: `() => Math.max(...args)`)",
    );
  }
  // Dev-mode closure detection. Production builds skip the scan entirely
  // — the heuristic is O(n) on the function source and not worth running
  // hot in user code.
  const isProd = typeof process !== "undefined" && (process.env?.NODE_ENV ?? "") === "production";
  if (!isProd) {
    const refs = detectClosureRefs(fnStr);
    if (refs.size > 0) {
      tynd.warn(
        `workers.spawn: function may reference closure vars [${[...refs].join(", ")}] — these will be undefined inside the Worker. Pass values via the run() input parameter instead.`,
      );
    }
  }
  return fnStr;
}

interface PendingWorkerCall {
  resolve(value: unknown): void;
  reject(error: Error): void;
}

class ManagedWebWorker implements WorkerHandle {
  private readonly pending = new Map<number, PendingWorkerCall>();
  private nextRunId = 0;
  private terminated = false;

  readonly id: number;
  private readonly worker: Worker;
  private readonly objectUrl?: string;

  constructor(id: number, worker: Worker, objectUrl?: string) {
    this.id = id;
    this.worker = worker;
    this.objectUrl = objectUrl;
    worker.onmessage = (event) => this.onMessage(event.data);
    worker.onerror = (event) => {
      event.preventDefault();
      this.dispose(new Error(event.message || "Web Worker crashed"));
    };
    worker.onmessageerror = () => {
      this.dispose(new Error("Web Worker response could not be decoded"));
    };
    workerIds.add(id);
  }

  run<Out = unknown, In = unknown>(input: In): Promise<Out> {
    if (this.terminated) return Promise.reject(new Error(`Worker ${this.id} terminated`));
    if (this.pending.size >= MAX_PENDING_WORKER_RUNS) {
      return Promise.reject(new Error(`Worker ${this.id} has too many pending tasks`));
    }
    const runId = this.allocateRunId();
    return new Promise<Out>((resolve, reject) => {
      this.pending.set(runId, { resolve: (value) => resolve(value as Out), reject });
      try {
        this.worker.postMessage({ runId, input });
      } catch (error) {
        this.pending.delete(runId);
        reject(error instanceof Error ? error : new Error(String(error)));
      }
    });
  }

  async terminate(): Promise<void> {
    this.dispose(new Error(`Worker ${this.id} terminated`));
  }

  private onMessage(raw: unknown): void {
    const frame = parseWorkerResponse(raw);
    if (!frame) {
      this.dispose(new Error("Web Worker sent an invalid response"));
      return;
    }
    const call = this.pending.get(frame.runId);
    if (!call) return;
    this.pending.delete(frame.runId);
    if (frame.ok) call.resolve(frame.value);
    else call.reject(new Error(typeof frame.value === "string" ? frame.value : "Worker failed"));
  }

  private allocateRunId(): number {
    for (;;) {
      this.nextRunId = this.nextRunId >= Number.MAX_SAFE_INTEGER ? 1 : this.nextRunId + 1;
      if (!this.pending.has(this.nextRunId)) return this.nextRunId;
    }
  }

  private dispose(error: Error): void {
    if (this.terminated) return;
    this.terminated = true;
    workerIds.delete(this.id);
    this.worker.terminate();
    if (this.objectUrl) URL.revokeObjectURL(this.objectUrl);
    for (const call of this.pending.values()) call.reject(error);
    this.pending.clear();
  }
}

function spawnWebWorker(script: string): WorkerHandle {
  const WorkerConstructor = (globalThis as { Worker?: typeof Worker }).Worker;
  if (typeof WorkerConstructor !== "function") {
    throw new Error("workers.spawn: Web Worker is unavailable in this renderer");
  }
  if (workerIds.size >= MAX_WORKERS) {
    throw new Error(`workers.spawn: worker limit reached (max ${MAX_WORKERS})`);
  }
  if (!script.trim()) throw new Error("workers.spawn: task source is empty");

  const id = allocateWorkerId();
  // A `blob:` URL, not `data:`: the default app CSP lists `blob:` under
  // `worker-src` but would refuse a `data:` worker script.
  const url = URL.createObjectURL(
    new Blob([createWorkerTaskScript(script)], { type: "application/javascript" }),
  );
  let worker: Worker;
  try {
    worker = new WorkerConstructor(url);
  } catch (error) {
    URL.revokeObjectURL(url);
    throw error;
  }
  return new ManagedWebWorker(id, worker, url);
}

/** Frontend bindings for sandboxed Web Workers. */
export const workers = {
  /**
   * Spawn a Worker that runs `taskFn` against the input passed to `run()`.
   * IMPORTANT: `taskFn` must be PURE — it is serialized via `.toString()`
   * and re-evaluated inside a fresh Worker context. Any closure over
   * external variables is silently lost (the captured names will be
   * `undefined` at runtime). Pass values via the `run()` input parameter
   * instead. Native functions (e.g. `Math.max`) cannot be serialized and
   * are rejected at spawn time.
   * String form (`workers.spawn("(input) => …")`) skips the closure check
   * — useful when you've pre-built the source yourself.
   */
  async spawn<In = unknown, Out = unknown>(
    taskFn: string | ((input: In) => Out | Promise<Out>),
  ): Promise<WorkerHandle> {
    const script = typeof taskFn === "string" ? taskFn : validateTaskFn(taskFn);
    return spawnWebWorker(script);
  },
  list(): Promise<number[]> {
    return Promise.resolve([...workerIds].sort((a, b) => a - b));
  },
};
