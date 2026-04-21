import { osCall } from "./_internal.js"

export type LogLevel = "debug" | "info" | "warn" | "error"

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
}
