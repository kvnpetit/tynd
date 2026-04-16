const c = {
  reset:  "\x1b[0m",
  bold:   "\x1b[1m",
  dim:    "\x1b[2m",
  green:  "\x1b[32m",
  cyan:   "\x1b[36m",
  yellow: "\x1b[33m",
  red:    "\x1b[31m",
  gray:   "\x1b[90m",
  white:  "\x1b[97m",
}

const prefix = `${c.cyan}${c.bold}vorn${c.reset}`

export const log = {
  info:    (...a: unknown[]) => console.log(`${prefix}  ${a.join(" ")}`),
  success: (...a: unknown[]) => console.log(`${prefix}  ${c.green}✓${c.reset} ${a.join(" ")}`),
  warn:    (...a: unknown[]) => console.log(`${prefix}  ${c.yellow}⚠${c.reset} ${a.join(" ")}`),
  error:   (...a: unknown[]) => console.error(`${prefix}  ${c.red}✗${c.reset} ${a.join(" ")}`),
  step:    (...a: unknown[]) => console.log(`${prefix}  ${c.gray}›${c.reset} ${a.join(" ")}`),
  blank:   ()                => console.log(),
  dim:     (...a: unknown[]) => console.log(`${c.dim}${a.join(" ")}${c.reset}`),
  bold:    (s: string)       => `${c.bold}${s}${c.reset}`,
  cyan:    (s: string)       => `${c.cyan}${s}${c.reset}`,
  green:   (s: string)       => `${c.green}${s}${c.reset}`,
  yellow:  (s: string)       => `${c.yellow}${s}${c.reset}`,
  gray:    (s: string)       => `${c.gray}${s}${c.reset}`,
}
