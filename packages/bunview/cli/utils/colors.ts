import pc from "picocolors";

export const c = pc;

type Level = "silent" | "error" | "warn" | "info" | "debug";
const LEVELS: Record<Level, number> = { silent: 0, error: 1, warn: 2, info: 3, debug: 4 };

function detectLevel(): Level {
  const argv = process.argv;
  if (argv.includes("--silent") || argv.includes("-s"))   return "silent";
  if (argv.includes("--verbose") || argv.includes("--debug")) return "debug";
  if (process.env.BUNVIEW_DEBUG) return "debug";
  return "info";
}

let threshold = LEVELS[detectLevel()];

export function setLogLevel(level: Level) { threshold = LEVELS[level]; }

/** Log helpers respecting `--silent` / `--verbose` / `BUNVIEW_DEBUG`. */
export const log = {
  info:  (msg: string) => { if (threshold >= LEVELS.info)  console.log (`${pc.cyan("[bunview]")} ${msg}`); },
  ok:    (msg: string) => { if (threshold >= LEVELS.info)  console.log (`${pc.green("[bunview] ✓")} ${msg}`); },
  warn:  (msg: string) => { if (threshold >= LEVELS.warn)  console.warn(`${pc.yellow("[bunview] ⚠")} ${msg}`); },
  error: (msg: string) => { if (threshold >= LEVELS.error) console.error(`${pc.red("[bunview] ✗")} ${msg}`); },
  step:  (msg: string) => { if (threshold >= LEVELS.info)  console.log (`${pc.dim("  →")} ${msg}`); },
  debug: (msg: string) => { if (threshold >= LEVELS.debug) console.log (`${pc.magenta("[debug]")} ${msg}`); },
};
