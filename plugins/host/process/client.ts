import { base64ToBytes, bytesToBase64, osCall, osDeadline, osOn } from "@tynd/core/client/bridge";

/** Options for `process.exec`. */
export interface ExecOptions {
  args?: string[];
  cwd?: string;
  env?: Record<string, string>;
  /**
   * String piped to the child's stdin. The byte stream the child
   * actually sees is determined by `inputEncoding` — UTF-8 by default.
   */
  input?: string;
  /**
   * How `input` is decoded before being piped to the child:
   *   - `"utf8"` (default) — write the raw UTF-8 bytes of `input`.
   *   - `"base64"` — decode `input` as RFC 4648 base64
   *     (with `=` padding), then pipe the resulting bytes. Binary
   *     payloads must use this; `input` is never auto-detected.
   */
  inputEncoding?: "utf8" | "base64";
  /**
   * How `stdout` and `stderr` come back from the host:
   *   - `"utf8"` (default) — `String::from_utf8_lossy`. Invalid
   *     UTF-8 sequences become U+FFFD replacement chars; binary
   *     output is corrupted.
   *   - `"base64"` — raw bytes encoded as RFC 4648 base64
   *     (with `=` padding). The only lossless choice for binary
   *     output (image bytes, gzip streams, etc.); decode with
   *     `atob` + `Uint8Array.from`.
   */
  outputEncoding?: "utf8" | "base64";
  /** Kill the child if it is still running after this many milliseconds. Defaults to 60s. */
  timeoutMs?: number;
  /** Per-stream stdout/stderr cap in bytes. Defaults to 16 MiB, host caps at 64 MiB. */
  maxOutputBytes?: number;
}

/** Captured output of a `process.exec` invocation. */
export interface ExecResult {
  code: number | null;
  stdout: string;
  stderr: string;
}

/** Options for `process.spawn`. */
export interface SpawnOptions {
  args?: string[];
  cwd?: string;
  env?: Record<string, string>;
}

/** Live handle to a child process spawned via `process.spawn`. */
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
  exit(): Promise<{ code: number | null; signal: string | null; error: string | null }>;
  /**
   * Send a kill signal to the child. The signal is best-effort — Unix
   * honours the requested signal via `nix::sys::signal::kill`, while
   * Windows ignores it and always uses `TerminateProcess` (the platform
   * has no equivalent of POSIX signals for foreign processes).
   */
  kill(signal?: "SIGTERM" | "SIGKILL" | "SIGINT"): Promise<void>;
}

interface ExitInfo {
  code: number | null;
  signal: string | null;
  /** Set when the host's `wait()` itself failed; distinguishes wait errors
   *  from a clean exit with `code: null`. */
  error: string | null;
}

/**
 * Wire-side stdout/stderr chunk shape. The host emits each chunk as a
 * JSON object `{ data: "<base64>" }`.
 */
function chunkFromEvent(raw: unknown): Uint8Array | null {
  if (typeof raw !== "object" || raw === null) return null;
  const data = (raw as { data?: unknown }).data;
  // RFC 4648 base64 with `=` padding, matching the host's base64 engine.
  return typeof data === "string" ? base64ToBytes(data) : null;
}

/**
 * Eager async iterator for one process byte stream. Construction subscribes
 * before the native activation handshake, so a child that writes and exits
 * immediately cannot outrun renderer listeners. The host emits stream EOF
 * from the same worker after its final chunk, preserving data-before-end.
 */
class ProcessByteIterator implements AsyncIterator<Uint8Array> {
  private readonly queue: Uint8Array[] = [];
  private pending: ((value: IteratorResult<Uint8Array>) => void) | null = null;
  private dataDispose: () => void = () => undefined;
  private endDispose: () => void = () => undefined;
  private endSeen = false;
  private finished = false;
  private disposed = false;

  constructor(eventName: string, endEventName: string) {
    try {
      this.dataDispose = osOn(eventName, (raw) => this.onData(raw));
      this.endDispose = osOn(endEventName, () => this.onEnd());
    } catch (error) {
      this.finished = true;
      this.dispose();
      throw error;
    }
  }

  next(): Promise<IteratorResult<Uint8Array>> {
    const value = this.queue.shift();
    if (value) return Promise.resolve({ value, done: false });
    if (this.finished || this.endSeen) {
      this.finished = true;
      this.dispose();
      return Promise.resolve({ value: undefined, done: true });
    }
    return new Promise((resolve) => {
      this.pending = resolve;
    });
  }

  async return(): Promise<IteratorResult<Uint8Array>> {
    this.finish();
    return { value: undefined, done: true };
  }

  async throw(error: unknown): Promise<IteratorResult<Uint8Array>> {
    this.finish();
    throw error;
  }

  private onData(raw: unknown): void {
    if (this.finished || this.disposed) return;
    const bytes = chunkFromEvent(raw);
    if (!bytes) return;
    const resolve = this.pending;
    if (!resolve) {
      this.queue.push(bytes);
      return;
    }
    this.pending = null;
    resolve({ value: bytes, done: false });
  }

  private onEnd(): void {
    if (this.disposed) return;
    this.endSeen = true;
    if (this.queue.length > 0) return;
    this.finished = true;
    const resolve = this.takePending();
    queueMicrotask(() => this.dispose());
    resolve?.({ value: undefined, done: true });
  }

  private finish(): void {
    this.finished = true;
    this.dispose();
    this.takePending()?.({ value: undefined, done: true });
  }

  private takePending(): ((value: IteratorResult<Uint8Array>) => void) | null {
    const resolve = this.pending;
    this.pending = null;
    return resolve;
  }

  private dispose(): void {
    if (this.disposed) return;
    this.disposed = true;
    try {
      this.dataDispose();
    } catch {
      // Best-effort cleanup must still attempt the EOF subscription.
    }
    try {
      this.endDispose();
    } catch {
      // A failed host unsubscribe leaves an inert callback because disposal is latched.
    }
  }
}

interface PreparedByteStream {
  iterable: AsyncIterable<Uint8Array>;
  dispose(): void;
}

function prepareByteStream(eventName: string, endEventName: string): PreparedByteStream {
  let first: ProcessByteIterator | null = new ProcessByteIterator(eventName, endEventName);
  return {
    iterable: {
      [Symbol.asyncIterator]() {
        const iterator = first ?? new ProcessByteIterator(eventName, endEventName);
        first = null;
        return iterator;
      },
    },
    dispose() {
      if (!first) return;
      void first.return();
      first = null;
    },
  };
}

interface PreparedSpawnedProcess {
  handle: SpawnedProcess;
  dispose(): void;
}

function makeProcessStdin(pid: number): SpawnedProcess["stdin"] {
  return {
    write(data: string | Uint8Array): Promise<void> {
      // The wire format is base64; strings become UTF-8 bytes first.
      const bytes = typeof data === "string" ? new TextEncoder().encode(data) : data;
      return osCall("process", "writeStdin", { pid, data: bytesToBase64(bytes) });
    },
    close(): Promise<void> {
      return osCall("process", "closeStdin", { pid });
    },
  };
}

interface PreparedProcessExit {
  promise: Promise<ExitInfo>;
  dispose(): void;
}

function parseExitInfo(raw: unknown): ExitInfo | null {
  if (typeof raw !== "object" || raw === null) return null;
  const data = raw as { code?: unknown; signal?: unknown; error?: unknown };
  return {
    code: typeof data.code === "number" ? data.code : null,
    signal: typeof data.signal === "string" ? data.signal : null,
    error: typeof data.error === "string" ? data.error : null,
  };
}

function prepareProcessExit(pid: number): PreparedProcessExit {
  let resolveExit!: (info: ExitInfo) => void;
  const promise = new Promise<ExitInfo>((resolve) => {
    resolveExit = resolve;
  });
  let settled = false;
  let rawDispose = (): void => undefined;
  let disposed = false;
  const dispose = (): void => {
    if (disposed) return;
    disposed = true;
    try {
      rawDispose();
    } catch {
      // The process handle is already inactive; keep cleanup best-effort.
    }
  };
  rawDispose = osOn(`process:exit:${pid}`, (raw) => {
    if (disposed || settled) return;
    const info = parseExitInfo(raw);
    if (!info) return;
    settled = true;
    resolveExit(info);
    queueMicrotask(dispose);
  });
  return { promise, dispose };
}

function makeSpawned(pid: number): PreparedSpawnedProcess {
  const stdout = prepareByteStream(`process:stdout:${pid}`, `process:stdout-end:${pid}`);
  let stderr: PreparedByteStream;
  try {
    stderr = prepareByteStream(`process:stderr:${pid}`, `process:stderr-end:${pid}`);
  } catch (error) {
    stdout.dispose();
    throw error;
  }
  let exit: PreparedProcessExit;
  try {
    exit = prepareProcessExit(pid);
  } catch (error) {
    stdout.dispose();
    stderr.dispose();
    throw error;
  }

  return {
    handle: {
      pid,
      stdout: stdout.iterable,
      stderr: stderr.iterable,
      stdin: makeProcessStdin(pid),
      exit: () => exit.promise,
      kill(signal: "SIGTERM" | "SIGKILL" | "SIGINT" = "SIGTERM"): Promise<void> {
        return osCall("process", "kill", { pid, signal });
      },
    },
    dispose() {
      stdout.dispose();
      stderr.dispose();
      exit.dispose();
    },
  };
}

// Convert the TS-side `args` field (the user-visible option name) to the
// host wire-name `argv` — `args` collides with the macro-generated
// `dispatch(args)` parameter inside #[tynd::handler].
function toWire<T extends { args?: string[] }>(opts?: T): Record<string, unknown> {
  if (!opts) return {};
  const { args, ...rest } = opts;
  return args === undefined ? { ...rest } : { ...rest, argv: args };
}

/** Frontend bindings for the host's `process` API (exec + streaming spawn). */
export const process = {
  /** Run a binary to completion and capture stdio + exit code. */
  exec(cmd: string, opts?: ExecOptions): Promise<ExecResult> {
    return osCall("process", "exec", { cmd, ...toWire(opts) }, osDeadline(opts?.timeoutMs));
  },
  /** Same as `exec` but spawns through the user's shell (parses pipes, redirects, etc.). */
  execShell(cmd: string, opts?: Omit<ExecOptions, "args">): Promise<ExecResult> {
    return osCall("process", "execShell", { cmd, ...opts }, osDeadline(opts?.timeoutMs));
  },
  /**
   * Spawn a long-running subprocess with streaming stdio. The returned
   * handle exposes async iterables for stdout/stderr, a stdin write
   * channel, an `exit()` promise, and `kill()`.
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
      ...toWire(opts),
    });
    let prepared: PreparedSpawnedProcess | undefined;
    try {
      // Data, EOF and exit listeners exist before native workers can emit.
      prepared = makeSpawned(pid);
      await osCall("process", "activate", { pid });
      return prepared.handle;
    } catch (error) {
      prepared?.dispose();
      await osCall("process", "kill", { pid, signal: "SIGKILL" }).catch(() => undefined);
      throw error;
    }
  },
};
