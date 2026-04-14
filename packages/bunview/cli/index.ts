#!/usr/bin/env bun
import { cac } from "cac";
import { fileURLToPath } from "url";
import path from "path";

import { runBuild } from "./commands/build";
import { runBuildAll } from "./commands/build-all";
import { runDev, runStart } from "./commands/dev";
import { runInit } from "./commands/init";
import { runClean } from "./commands/clean";
import { runCreate } from "./commands/create";
import { runUpgrade } from "./commands/upgrade";
import { runInfo } from "./commands/info";
import { runValidate } from "./commands/validate";
import { log } from "./utils/colors";
import { maybeNotifyUpdate } from "./utils/update-notifier";

async function readVersion(): Promise<string> {
  const pkgPath = path.join(fileURLToPath(new URL("../", import.meta.url)), "package.json");
  try {
    const pkg = await Bun.file(pkgPath).json() as { version: string };
    return pkg.version;
  } catch { return "0.0.0"; }
}

/** Convert camelCase → kebab-case so flags round-trip through cac without losing their original form. */
function camelToKebab(s: string): string {
  return s.replace(/([a-z0-9])([A-Z])/g, "$1-$2").toLowerCase();
}

function optsToArgs(opts: Record<string, unknown>): string[] {
  const out: string[] = [];
  for (const [k, v] of Object.entries(opts)) {
    if (k === "--" || k === "_") continue;
    const flag = camelToKebab(k);
    if (v === true) out.push(`--${flag}`);
    else if (v === false || v === undefined || v === null) continue;
    else out.push(`--${flag}=${v}`);
  }
  return out;
}

// Short command aliases — common shortcuts found in modern CLIs (pnpm, cargo).
// We rewrite argv before cac parses it.
const ALIASES: Record<string, string> = {
  b: "build",
  d: "dev",
  s: "start",
  i: "init",
  c: "clean",
};
if (process.argv[2] && ALIASES[process.argv[2]]) {
  process.argv[2] = ALIASES[process.argv[2]]!;
}

const cli = cac("bunview");

cli.command("init", "Scaffold bunview into an existing project (auto-detects framework)")
  .option("--no-install", "skip automatic `bun install`")
  .option("--force, -f",  "overwrite existing bunview.config.ts")
  .option("--dry-run",    "preview changes without writing files")
  .action((opts) => runInit(optsToArgs(opts)));

cli.command("create [name]", "Create a fresh bunview project from a template")
  .option("--template, -t <name>", "template: vanilla | react | vue | svelte | solid")
  .option("--yes, -y", "non-interactive: use defaults without prompts")
  .action((name: string | undefined, opts) => {
    const args = optsToArgs(opts);
    if (name) args.unshift(name);
    return runCreate(args);
  });

cli.command("dev", "Run app in watch/reload mode (auto-detects Vite/CRA/Angular/…)")
  .option("--port <n>",      "dev server port")
  .option("--host <h>",      "dev server host")
  .option("--open",          "open browser once dev server is ready")
  .option("--inspect",       "enable Bun inspector for the backend")
  .option("--backend-only",  "skip frontend dev server")
  .option("--frontend-only", "run dev server only, no backend")
  .action((opts) => runDev({
    port:         opts.port ? Number(opts.port) : undefined,
    host:         typeof opts.host === "string" ? opts.host : undefined,
    open:         opts.open === true,
    inspect:      opts.inspect === true,
    backendOnly:  opts.backendOnly === true,
    frontendOnly: opts.frontendOnly === true,
  }));

cli.command("start [...args]", "Start the built app — forwards extra args to your app")
  .action((args: string[]) => runStart(args ?? []));

cli.command("build [target]", "Build standalone binary + portable + installer")
  .option("--all",             "build for all 6 desktop targets")
  .option("--windows",         "windows-x64 (Intel/AMD)")
  .option("--windows-x64",     "windows-x64")
  .option("--windows-arm64",   "Surface / Snapdragon X")
  .option("--linux",           "linux-x64 (Intel/AMD)")
  .option("--linux-x64",       "linux-x64")
  .option("--linux-arm64",     "Raspberry Pi / AWS Graviton")
  .option("--macos",           "auto (x64 or arm64)")
  .option("--macos-x64",       "Intel Mac")
  .option("--macos-arm64",     "Apple Silicon (M1/M2/M3/M4)")
  .option("--skip-portable",   "skip portable archive generation")
  .option("--skip-installer",  "skip installer generation (NSIS/DMG/deb)")
  .action((target: string | undefined, opts) => {
    if (opts.all) return runBuildAll([]);
    const args = optsToArgs(opts);
    if (target) args.unshift(target);
    return runBuild(args);
  });

cli.command("clean", "Remove release/, dist/, .bunview-tmp")
  .option("--yes, -y", "skip confirmation prompt")
  .action((opts) => runClean(optsToArgs(opts)));

cli.command("upgrade", "Upgrade bunview to the latest version from npm")
  .option("--yes, -y", "skip confirmation prompt")
  .action((opts) => runUpgrade(optsToArgs(opts)));

cli.command("info", "Print project + runtime information")
  .option("--json", "output machine-readable JSON")
  .action((opts) => runInfo(optsToArgs(opts)));

cli.command("validate", "Validate bunview.config.ts + referenced files (useful in CI)")
  .option("--json", "output machine-readable JSON")
  .action((opts) => runValidate(optsToArgs(opts)));

// Global flags declared for --help visibility. Parsed directly in the relevant modules.
cli.option("--config, -c <path>", "use a specific bunview.config.ts path");
cli.option("--silent, -s",        "suppress info/ok output (errors still shown)");
cli.option("--verbose",           "print debug output (same as BUNVIEW_DEBUG=1)");

cli.help();
cli.version(await readVersion());

function distance(a: string, b: string): number {
  const dp: number[][] = Array.from({ length: a.length + 1 }, () => Array(b.length + 1).fill(0));
  for (let i = 0; i <= a.length; i++) dp[i]![0] = i;
  for (let j = 0; j <= b.length; j++) dp[0]![j] = j;
  for (let i = 1; i <= a.length; i++) {
    for (let j = 1; j <= b.length; j++) {
      dp[i]![j] = a[i - 1] === b[j - 1]
        ? dp[i - 1]![j - 1]!
        : 1 + Math.min(dp[i - 1]![j]!, dp[i]![j - 1]!, dp[i - 1]![j - 1]!);
    }
  }
  return dp[a.length]![b.length]!;
}

const KNOWN = [
  "init", "create", "dev", "start", "build",
  "clean", "validate", "info", "upgrade",
];

const HELP_FLAGS = new Set(["-h", "--help", "-v", "--version"]);

// Global safety nets: friendly messages instead of raw stack traces.
process.on("unhandledRejection", (reason) => {
  log.error(`Unhandled promise rejection: ${reason instanceof Error ? reason.message : String(reason)}`);
  if (process.env.BUNVIEW_DEBUG) console.error(reason);
  else console.error(`  Run with BUNVIEW_DEBUG=1 or --verbose for the full stack.`);
  process.exit(1);
});
process.on("uncaughtException", (err) => {
  log.error(`Uncaught exception: ${err.message}`);
  if (process.env.BUNVIEW_DEBUG) console.error(err);
  else console.error(`  Run with BUNVIEW_DEBUG=1 or --verbose for the full stack.`);
  process.exit(1);
});

// Fire-and-forget update check (cached 24h, opt-out via CI or BUNVIEW_NO_UPDATE_CHECK).
const updateCheck = maybeNotifyUpdate().catch(() => {});

try {
  cli.parse(process.argv, { run: false });

  // Skip unknown-command check when a help/version flag was passed (cac handles those).
  const argv = process.argv.slice(2);
  const hasHelpOrVersion = argv.some((a) => HELP_FLAGS.has(a));

  if (!cli.matchedCommand && !hasHelpOrVersion) {
    const raw = argv[0];
    if (raw && !raw.startsWith("-")) {
      const best = KNOWN.map((k) => ({ k, d: distance(raw, k) }))
        .filter((s) => s.d <= 3)
        .sort((a, b) => a.d - b.d)[0];
      log.error(`Unknown command: "${raw}"`);
      if (best) console.log(`  Did you mean \`bunview ${best.k}\`?`);
      else console.log(`  Run \`bunview --help\` to list commands.`);
      process.exit(1);
    }
  }

  await cli.runMatchedCommand();
  // Let the update check finish (max 2s due to AbortSignal) before exiting.
  await Promise.race([updateCheck, new Promise((r) => setTimeout(r, 100))]);
} catch (err) {
  log.error(err instanceof Error ? err.message : String(err));
  if (process.env.BUNVIEW_DEBUG) console.error(err);
  process.exit(1);
}
