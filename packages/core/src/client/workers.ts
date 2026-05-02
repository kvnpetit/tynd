import { tynd } from "../logger";
import { osCall } from "./_internal";

export interface WorkerHandle {
  id: number;
  run<Out = unknown, In = unknown>(input: In): Promise<Out>;
  terminate(): Promise<void>;
}

let bunWorkerSeq = 0;

/**
 * Identifiers that always resolve in a fresh Worker context — both standard
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

function spawnBunWorker(script: string): WorkerHandle {
  const id = ++bunWorkerSeq;
  const wrapped = `
    const __fn = (${script});
    self.onmessage = async (e) => {
      const { rid, input } = e.data
      try {
        const result = await __fn(input)
        postMessage({ rid, result })
      } catch (err) {
        postMessage({ rid, error: String((err && err.message) || err) })
      }
    }
  `;
  const url = `data:application/javascript,${encodeURIComponent(wrapped)}`;
  const w = new (globalThis as unknown as { Worker: new (u: string) => Worker }).Worker(url);
  const pending = new Map<number, { resolve: (v: unknown) => void; reject: (e: Error) => void }>();
  let rseq = 0;

  w.onmessage = (e: MessageEvent) => {
    const { rid, result, error } = e.data as { rid: number; result?: unknown; error?: string };
    const p = pending.get(rid);
    if (!p) return;
    pending.delete(rid);
    if (error) p.reject(new Error(error));
    else p.resolve(result);
  };

  return {
    id,
    run<Out, In>(input: In): Promise<Out> {
      const rid = ++rseq;
      return new Promise<Out>((resolve, reject) => {
        pending.set(rid, { resolve: resolve as (v: unknown) => void, reject });
        w.postMessage({ rid, input });
      });
    },
    terminate() {
      w.terminate();
      // Without this, any awaited run() outstanding at terminate time would
      // hang forever — the worker is dead, nothing resolves the promise.
      const err = new Error("Worker terminated");
      for (const p of pending.values()) p.reject(err);
      pending.clear();
      return Promise.resolve();
    },
  };
}

function isBunWorkerAvailable(): boolean {
  return (
    typeof (globalThis as { Worker?: unknown }).Worker === "function" &&
    !(globalThis as { __tynd_lite__?: unknown }).__tynd_lite__
  );
}

export const workers = {
  /**
   * Spawn a Worker that runs `taskFn` against the input passed to `run()`.
   *
   * IMPORTANT: `taskFn` must be PURE — it is serialized via `.toString()`
   * and re-evaluated inside a fresh Worker context. Any closure over
   * external variables is silently lost (the captured names will be
   * `undefined` at runtime). Pass values via the `run()` input parameter
   * instead. Native functions (e.g. `Math.max`) cannot be serialized and
   * are rejected at spawn time.
   *
   * String form (`workers.spawn("(input) => …")`) skips the closure check
   * — useful when you've pre-built the source yourself.
   */
  async spawn(taskFn: string | ((input: unknown) => unknown)): Promise<WorkerHandle> {
    const script = typeof taskFn === "string" ? taskFn : validateTaskFn(taskFn);
    if (isBunWorkerAvailable()) return spawnBunWorker(script);
    const { id } = await osCall<{ id: number }>("workers", "spawn", { script });
    return {
      id,
      run: <Out, In>(input: In) => osCall<Out>("workers", "run", { id, input }),
      terminate: () => osCall<void>("workers", "terminate", { id }),
    };
  },
  list(): Promise<number[]> {
    if (isBunWorkerAvailable()) return Promise.resolve([]);
    return osCall("workers", "list");
  },
};
