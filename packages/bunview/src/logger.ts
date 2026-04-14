import path from "path";
import fs from "fs";

export type LogLevel = "debug" | "info" | "warn" | "error";
const LEVELS: Record<LogLevel, number> = { debug: 0, info: 1, warn: 2, error: 3 };

export interface LoggerOptions {
  /** Log directory — required for file output. */
  dir?: string;
  /** Minimum level to record. Default: "info". */
  level?: LogLevel;
  /** Max single log file size in bytes before rotating. Default: 5 MB. */
  maxSize?: number;
  /** Number of rotated files to keep. Default: 3. */
  maxFiles?: number;
  /** Mirror output to the console. Default: `true`. */
  console?: boolean;
}

/**
 * Lightweight structured logger with file rotation.
 * Each line is `<ISO-timestamp> [LEVEL] <message>`; optional JSON metadata
 * is appended as trailing JSON.
 */
export class Logger {
  private readonly dir:       string | null;
  private readonly threshold: number;
  private readonly maxSize:   number;
  private readonly maxFiles:  number;
  private readonly useConsole: boolean;
  private readonly filePath:  string | null;

  constructor(opts: LoggerOptions = {}) {
    this.dir = opts.dir ?? null;
    this.threshold = LEVELS[opts.level ?? "info"];
    this.maxSize   = opts.maxSize  ?? 5 * 1024 * 1024;
    this.maxFiles  = opts.maxFiles ?? 3;
    this.useConsole = opts.console ?? true;
    this.filePath = this.dir ? path.join(this.dir, "app.log") : null;
    if (this.dir) { try { fs.mkdirSync(this.dir, { recursive: true }); } catch {} }
  }

  private rotate(): void {
    if (!this.filePath) return;
    try {
      const stat = fs.statSync(this.filePath);
      if (stat.size < this.maxSize) return;
    } catch { return; }

    // Shift older files: app.log.(n-1) → app.log.n, then app.log → app.log.1
    for (let i = this.maxFiles - 1; i >= 1; i--) {
      const from = `${this.filePath}.${i}`;
      const to   = `${this.filePath}.${i + 1}`;
      try { fs.renameSync(from, to); } catch {}
    }
    try { fs.renameSync(this.filePath, `${this.filePath}.1`); } catch {}
  }

  private write(level: LogLevel, message: string, meta?: unknown): void {
    if (LEVELS[level] < this.threshold) return;
    const line = `${new Date().toISOString()} [${level.toUpperCase()}] ${message}` +
      (meta !== undefined ? ` ${JSON.stringify(meta)}` : "");

    if (this.useConsole) {
      const fn = level === "error" ? console.error
              : level === "warn"  ? console.warn
              : level === "debug" ? console.debug : console.log;
      fn(line);
    }
    if (this.filePath) {
      this.rotate();
      try { fs.appendFileSync(this.filePath, line + "\n"); } catch {}
    }
  }

  debug(msg: string, meta?: unknown) { this.write("debug", msg, meta); }
  info (msg: string, meta?: unknown) { this.write("info",  msg, meta); }
  warn (msg: string, meta?: unknown) { this.write("warn",  msg, meta); }
  error(msg: string, meta?: unknown) { this.write("error", msg, meta); }
}
