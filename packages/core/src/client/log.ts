/**
 * File logger client. Mirrors the host's `os.log.*` dispatch table.
 * The host writes JSON-lines to disk with size and UTC time rotation. Calling
 * `log.configure({...})` is opt-in: until then any `log.write()` is a
 * no-op above the default `info` threshold and starts a file at the
 * platform-default path (`<cache>/<exe>/app.log`) on first write.
 * `captureConsole()` keeps the previous plugin's console-as-file
 * convenience: it patches `console.*` so DevTools still works, plus
 * forwards each call to `log.write` so the file sink mirrors the
 * console.
 */

import { osCall } from "./_internal";

export type LogLevel = "debug" | "info" | "warn" | "error";
export type LogRotation = "size" | "daily" | "hourly" | "both";

export interface LogConfigureOptions {
  level?: LogLevel;
  /** Absolute path to the log file. Default: `<cache>/<exe>/app.log`. */
  file?: string;
  /** Rotate when the active file reaches this size (bytes). Default 5 MiB. */
  maxBytes?: number;
  /** How many rotated copies to keep. Default 3. */
  maxFiles?: number;
  /**
   * Rotation trigger. `both` combines `maxBytes` and daily UTC rotation.
   * Default: `size`.
   */
  rotation?: LogRotation;
}

let consoleRestore: (() => void) | null = null;

function write(level: LogLevel, message: string, fields?: Record<string, unknown>): Promise<void> {
  return osCall("log", "write", { level, message, fields }) as Promise<void>;
}

export const log = {
  configure(options: LogConfigureOptions): Promise<void> {
    return osCall("log", "configure", options) as Promise<void>;
  },
  path(): Promise<string> {
    return osCall("log", "path", null);
  },
  debug(message: string, fields?: Record<string, unknown>): Promise<void> {
    return write("debug", message, fields);
  },
  info(message: string, fields?: Record<string, unknown>): Promise<void> {
    return write("info", message, fields);
  },
  warn(message: string, fields?: Record<string, unknown>): Promise<void> {
    return write("warn", message, fields);
  },
  error(message: string, fields?: Record<string, unknown>): Promise<void> {
    return write("error", message, fields);
  },

  /**
   * Route `console.*` through `log.write`. The original console methods
   * still run (DevTools stays usable); this just adds the file sink.
   * Idempotent.
   */
  captureConsole(): void {
    if (consoleRestore || typeof console === "undefined") return;
    const original = {
      log: console.log,
      info: console.info,
      warn: console.warn,
      error: console.error,
      debug: console.debug,
    };
    const emit = (level: LogLevel, args: unknown[]) => {
      const message = args.map(formatArg).join(" ");
      // Route failures to the original console so captureConsole stays a
      // non-destructive overlay: DevTools never goes silent even if the
      // file sink can't be reached.
      write(level, message).catch((err) => original.error.call(console, "[@tynd/core/log]", err));
    };
    console.log = (...args) => {
      original.log.apply(console, args);
      emit("info", args);
    };
    console.info = (...args) => {
      original.info.apply(console, args);
      emit("info", args);
    };
    console.warn = (...args) => {
      original.warn.apply(console, args);
      emit("warn", args);
    };
    console.error = (...args) => {
      original.error.apply(console, args);
      emit("error", args);
    };
    console.debug = (...args) => {
      original.debug.apply(console, args);
      emit("debug", args);
    };
    consoleRestore = () => {
      console.log = original.log;
      console.info = original.info;
      console.warn = original.warn;
      console.error = original.error;
      console.debug = original.debug;
      consoleRestore = null;
    };
  },
  restoreConsole(): void {
    consoleRestore?.();
  },
};

/** Cap on serialized size of a single `console.*` argument — guards against
 * a user logging a 50 MB object + freezing the renderer on JSON.stringify. */
const FORMAT_ARG_MAX = 8 * 1024;

function formatArg(arg: unknown): string {
  if (typeof arg === "string") return arg;
  if (arg instanceof Error) return arg.stack ?? arg.message;
  try {
    const s = JSON.stringify(arg);
    if (s === undefined) return String(arg);
    return s.length > FORMAT_ARG_MAX ? `${s.slice(0, FORMAT_ARG_MAX)}… (${s.length} chars)` : s;
  } catch {
    return String(arg);
  }
}
