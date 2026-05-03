import { osCall } from "./_internal";

export interface ExecOptions {
  args?: string[];
  cwd?: string;
  env?: Record<string, string>;
  input?: string;
}

export interface ExecResult {
  code: number | null;
  stdout: string;
  stderr: string;
}

export interface SpawnOptions {
  args?: string[];
  cwd?: string;
  env?: Record<string, string>;
}

export interface SpawnedProcess {
  /** Subprocess PID assigned by the OS. Stable until exit. */
  readonly pid: number;
  /**
   * Async iterable of stdout chunks (`Uint8Array`). Iteration completes
   * when the child closes stdout (typically at exit). Use a `TextDecoder`
   * to assemble UTF-8 lines.
   */
  readonly stdout: AsyncIterable<Uint8Array>;
  /** Async iterable of stderr chunks (`Uint8Array`). */
  readonly stderr: AsyncIterable<Uint8Array>;
  /** Write to / close the child's stdin pipe. */
  readonly stdin: {
    /** Append `data` to the child's stdin. UTF-8-encoded if a `string`. */
    write(data: string | Uint8Array): Promise<void>;
    /** Close the write end of the pipe (the child sees EOF on stdin). */
    close(): Promise<void>;
  };
  /**
   * Resolves with the exit metadata once the child terminates. Cached —
   * calling `exit()` after termination returns the same result without
   * re-awaiting.
   */
  exit(): Promise<{ code: number | null; signal: string | null }>;
  /**
   * Send a kill signal to the child. The signal is best-effort — Windows
   * always uses `TerminateProcess` regardless of `signal`, and on Unix
   * the host currently maps every signal to SIGKILL pending a libc::kill
   * pass.
   */
  kill(signal?: "SIGTERM" | "SIGKILL" | "SIGINT"): Promise<void>;
}

interface ExitInfo {
  code: number | null;
  signal: string | null;
}

/**
 * Wire-side stdout/stderr chunk shape. The host emits each chunk as a
 * JSON object `{ data: "<base64>" }` — 25× smaller on the wire than
 * the legacy `Number[]` array (1 byte = 1-3 ASCII chars + comma) and
 * ~50× cheaper to serialize on the host. The legacy array shape is
 * still accepted (host built with `process-bytes-array` feature) so a
 * mismatched core/host pair degrades gracefully instead of dropping
 * every chunk.
 */
function chunkFromEvent(raw: unknown): Uint8Array | null {
  if (typeof raw !== "object" || raw === null) return null;
  const data = (raw as { data?: unknown }).data;
  // Default base64 wire format — `atob` decodes the standard alphabet
  // (with `=` padding), which is what `base64::engine::general_purpose::STANDARD`
  // emits on the host.
  if (typeof data === "string") {
    const binary = atob(data);
    const out = new Uint8Array(binary.length);
    for (let i = 0; i < binary.length; i++) out[i] = binary.charCodeAt(i);
    return out;
  }
  // Legacy `Number[]` shape — preserved for forks running the host with
  // `--features process-bytes-array`. New code should leave it off.
  if (Array.isArray(data)) return Uint8Array.from(data as number[]);
  return null;
}

/**
 * Build an async iterable that yields every `data` chunk arriving on
 * `eventName` until `done()` returns true. Used by both stdout and stderr.
 *
 * The stream is push-based on the host side (events arrive whether or not
 * the consumer is currently `await`-ing) — we buffer in a queue + a
 * pending resolver to avoid losing chunks during back-to-back reads. A
 * single tail handler is registered per stream to avoid re-binding to the
 * shim on each iteration.
 */
function streamFromEvent(eventName: string, isDone: () => boolean): AsyncIterable<Uint8Array> {
  return {
    [Symbol.asyncIterator]() {
      const queue: Uint8Array[] = [];
      let pending: ((value: IteratorResult<Uint8Array>) => void) | null = null;
      let finished = false;

      const flushDoneIfReady = () => {
        if (!isDone() || queue.length > 0) return;
        finished = true;
        const resolve = pending;
        pending = null;
        resolve?.({ value: undefined, done: true });
      };

      const dispose = window.__tynd__.os_on(eventName, (raw) => {
        if (finished) return;
        const bytes = chunkFromEvent(raw);
        if (!bytes) return;
        if (pending) {
          const resolve = pending;
          pending = null;
          resolve({ value: bytes, done: false });
          return;
        }
        queue.push(bytes);
      });

      // Poll for exit at the same cadence as event arrival — cheap because
      // `isDone()` is just a closure read of the cached exit info.
      const exitWatch = setInterval(flushDoneIfReady, 50);

      return {
        next() {
          if (queue.length > 0) {
            const value = queue.shift()!;
            return Promise.resolve({ value, done: false });
          }
          if (finished || isDone()) {
            finished = true;
            return Promise.resolve({ value: undefined, done: true });
          }
          return new Promise<IteratorResult<Uint8Array>>((resolve) => {
            pending = resolve;
          });
        },
        async return() {
          finished = true;
          dispose();
          clearInterval(exitWatch);
          return { value: undefined, done: true };
        },
        async throw(err) {
          finished = true;
          dispose();
          clearInterval(exitWatch);
          throw err;
        },
      };
    },
  };
}

function makeSpawned(pid: number): SpawnedProcess {
  let exitInfo: ExitInfo | null = null;
  // Single Promise resolved once on the first exit event so multiple
  // `exit()` callers all observe the same value.
  let exitResolve: ((info: ExitInfo) => void) | null = null;
  const exitPromise = new Promise<ExitInfo>((resolve) => {
    exitResolve = resolve;
  });
  const exitDispose = window.__tynd__.os_on(`process:exit:${pid}`, (raw) => {
    if (typeof raw !== "object" || raw === null) return;
    const data = raw as { code?: unknown; signal?: unknown };
    const info: ExitInfo = {
      code: typeof data.code === "number" ? data.code : null,
      signal: typeof data.signal === "string" ? data.signal : null,
    };
    if (exitInfo) return;
    exitInfo = info;
    exitResolve?.(info);
    exitDispose();
  });

  const isExited = () => exitInfo !== null;

  return {
    pid,
    stdout: streamFromEvent(`process:stdout:${pid}`, isExited),
    stderr: streamFromEvent(`process:stderr:${pid}`, isExited),
    stdin: {
      write(data: string | Uint8Array): Promise<void> {
        // The host accepts both string and a number[] form. Convert
        // Uint8Array to a plain array so the JSON wire stays printable.
        const wire = typeof data === "string" ? data : Array.from(data as Uint8Array);
        return osCall("process", "writeStdin", { pid, data: wire });
      },
      close(): Promise<void> {
        return osCall("process", "closeStdin", { pid });
      },
    },
    exit() {
      return exitPromise;
    },
    kill(signal: "SIGTERM" | "SIGKILL" | "SIGINT" = "SIGTERM"): Promise<void> {
      return osCall("process", "kill", { pid, signal });
    },
  };
}

export const process = {
  exec(cmd: string, opts?: ExecOptions): Promise<ExecResult> {
    return osCall("process", "exec", { cmd, ...opts });
  },
  execShell(cmd: string, opts?: Omit<ExecOptions, "args">): Promise<ExecResult> {
    return osCall("process", "execShell", { cmd, ...opts });
  },
  /**
   * Spawn a long-running subprocess with streaming stdio. The returned
   * handle exposes async iterables for stdout/stderr, a stdin write
   * channel, an `exit()` promise, and `kill()`.
   *
   * @example
   *   const proc = await process.spawn("git", { args: ["log", "--oneline"] });
   *   const dec = new TextDecoder();
   *   for await (const chunk of proc.stdout) {
   *     console.log(dec.decode(chunk));
   *   }
   *   const { code } = await proc.exit();
   */
  async spawn(cmd: string, opts?: SpawnOptions): Promise<SpawnedProcess> {
    const { pid } = await osCall<{ pid: number }>("process", "spawn", {
      cmd,
      ...opts,
    });
    return makeSpawned(pid);
  },
};
