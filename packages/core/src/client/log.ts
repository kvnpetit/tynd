import { osCall } from "./_internal.js"

export type LogLevel = "debug" | "info" | "warn" | "error"

let consoleRestore: (() => void) | null = null

export interface LogConfigureOptions {
  /** Minimum level written to the file — lower levels are dropped. */
  level?: LogLevel
  /** Override the log file path. Default: `<cache_dir>/<app>/app.log`. */
  file?: string
  /** Rotate when the active file reaches this many bytes (default 5 MiB). */
  maxBytes?: number
  /** How many numbered backups to keep (default 3 — `.log.1` .. `.log.3`). */
  maxFiles?: number
}

function write(level: LogLevel, message: string, fields?: Record<string, unknown>): Promise<void> {
  return osCall("log", "write", { level, message, fields })
}

export const log = {
  configure(options: LogConfigureOptions): Promise<void> {
    return osCall("log", "configure", options)
  },
  /** Absolute path of the currently active log file. */
  path(): Promise<string> {
    return osCall("log", "path")
  },
  debug(message: string, fields?: Record<string, unknown>): Promise<void> {
    return write("debug", message, fields)
  },
  info(message: string, fields?: Record<string, unknown>): Promise<void> {
    return write("info", message, fields)
  },
  warn(message: string, fields?: Record<string, unknown>): Promise<void> {
    return write("warn", message, fields)
  },
  error(message: string, fields?: Record<string, unknown>): Promise<void> {
    return write("error", message, fields)
  },

  /**
   * Route `console.log` / `.info` / `.warn` / `.error` / `.debug` through
   * `log.write`. The original methods still run so DevTools remain usable;
   * this just adds a second sink on the log file. Idempotent.
   */
  captureConsole(): void {
    if (consoleRestore || typeof console === "undefined") return
    const original = {
      log: console.log,
      info: console.info,
      warn: console.warn,
      error: console.error,
      debug: console.debug,
    }
    const emit = (level: LogLevel, args: unknown[]) => {
      const message = args.map(formatArg).join(" ")
      void write(level, message)
    }
    console.log = (...args) => {
      original.log.apply(console, args)
      emit("info", args)
    }
    console.info = (...args) => {
      original.info.apply(console, args)
      emit("info", args)
    }
    console.warn = (...args) => {
      original.warn.apply(console, args)
      emit("warn", args)
    }
    console.error = (...args) => {
      original.error.apply(console, args)
      emit("error", args)
    }
    console.debug = (...args) => {
      original.debug.apply(console, args)
      emit("debug", args)
    }
    consoleRestore = () => {
      console.log = original.log
      console.info = original.info
      console.warn = original.warn
      console.error = original.error
      console.debug = original.debug
      consoleRestore = null
    }
  },
  /** Undo `captureConsole`. Safe to call if capture isn't active. */
  restoreConsole(): void {
    consoleRestore?.()
  },
}

function formatArg(arg: unknown): string {
  if (typeof arg === "string") return arg
  if (arg instanceof Error) return arg.stack ?? arg.message
  try {
    return JSON.stringify(arg)
  } catch {
    return String(arg)
  }
}
