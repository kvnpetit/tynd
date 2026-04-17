// ANSI is emitted only when stdout is a TTY and NO_COLOR is unset.
// Respect https://no-color.org so CI logs stay greppable.
const useColor = Boolean(process.stdout.isTTY) && !process.env["NO_COLOR"]

const ansi = (code: string) => (useColor ? code : "")
const c = {
  reset: ansi("\x1b[0m"),
  bold: ansi("\x1b[1m"),
  dim: ansi("\x1b[2m"),
  green: ansi("\x1b[32m"),
  cyan: ansi("\x1b[36m"),
  yellow: ansi("\x1b[33m"),
  red: ansi("\x1b[31m"),
  gray: ansi("\x1b[90m"),
  white: ansi("\x1b[97m"),
}

const prefix = `${c.cyan}${c.bold}tynd${c.reset}`

export type LogLevel = "quiet" | "normal" | "verbose"

let currentLevel: LogLevel = "normal"

export function setLogLevel(level: LogLevel): void {
  currentLevel = level
}

export function getLogLevel(): LogLevel {
  return currentLevel
}

const showStep = () => currentLevel !== "quiet"
const showInfo = () => currentLevel !== "quiet"
const showDebug = () => currentLevel === "verbose"

export const log = {
  info: (...a: unknown[]) => showInfo() && console.log(`${prefix}  ${a.join(" ")}`),
  success: (...a: unknown[]) =>
    showInfo() && console.log(`${prefix}  ${c.green}✓${c.reset} ${a.join(" ")}`),
  warn: (...a: unknown[]) => console.log(`${prefix}  ${c.yellow}⚠${c.reset} ${a.join(" ")}`),
  error: (...a: unknown[]) => console.error(`${prefix}  ${c.red}✗${c.reset} ${a.join(" ")}`),
  step: (...a: unknown[]) =>
    showStep() && console.log(`${prefix}  ${c.gray}›${c.reset} ${a.join(" ")}`),
  debug: (...a: unknown[]) =>
    showDebug() && console.log(`${prefix}  ${c.dim}· ${a.join(" ")}${c.reset}`),
  blank: () => showInfo() && console.log(),
  dim: (...a: unknown[]) => showInfo() && console.log(`${c.dim}${a.join(" ")}${c.reset}`),
  /**
   * Print an error followed by a suggested next step. Keeps the two lines
   * visually paired so users always see the fix after the cause.
   */
  hint: (message: string, next: string) => {
    console.error(`${prefix}  ${c.red}✗${c.reset} ${message}`)
    console.error(`${c.dim}  -> ${next}${c.reset}`)
  },
  bold: (s: string) => `${c.bold}${s}${c.reset}`,
  cyan: (s: string) => `${c.cyan}${s}${c.reset}`,
  green: (s: string) => `${c.green}${s}${c.reset}`,
  yellow: (s: string) => `${c.yellow}${s}${c.reset}`,
  gray: (s: string) => `${c.gray}${s}${c.reset}`,
}
