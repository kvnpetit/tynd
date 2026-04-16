#!/usr/bin/env bun
import { cac } from "cac"
import { build } from "./commands/build.ts"
import { clean } from "./commands/clean.ts"
import { create, FRAMEWORKS, type Framework } from "./commands/create.ts"
import { dev } from "./commands/dev.ts"
import { info } from "./commands/info.ts"
import { init } from "./commands/init.ts"
import { upgrade } from "./commands/upgrade.ts"
import { validate } from "./commands/validate.ts"
import { ConfigError } from "./lib/config.ts"
import { log, setLogLevel } from "./lib/logger.ts"
import { VERSION } from "./lib/version.ts"

const cli = cac("tynd")

const frameworkValues = FRAMEWORKS.map((f) => f.value).join(" | ")

cli.option("--verbose", "Show debug-level logs")
cli.option("--quiet", "Suppress everything except errors")

cli
  .command("create [name]", "Scaffold a new Tynd project")
  .option("-f, --framework <fw>", `Frontend framework: ${frameworkValues}`)
  .option("-r, --runtime <runtime>", "Runtime: full | lite")
  .option("--no-install", "Skip dependency installation prompt")
  .action(
    async (
      name: string | undefined,
      opts: { framework?: string; runtime?: string; install?: boolean },
    ) => {
      const { text, select } = await import("./lib/prompt.ts")

      // Resolve each arg independently — prompt only for what is missing
      const resolvedName =
        name ??
        (await (async () => {
          log.blank()
          log.info("Create a new Tynd project")
          log.blank()
          return text("Project name", "my-app")
        })())

      const resolvedFw: Framework =
        (FRAMEWORKS.find((f) => f.value === opts.framework)?.value as Framework | undefined) ??
        (await select<Framework>("Frontend framework", [...FRAMEWORKS]))

      const resolvedRuntime: "full" | "lite" =
        opts.runtime === "full" || opts.runtime === "lite"
          ? opts.runtime
          : await select<"full" | "lite">("Backend runtime", [
              { value: "full", label: "full", hint: "full — Node.js APIs, npm ecosystem" },
              {
                value: "lite",
                label: "lite",
                hint: "smaller binary (~10 MB), no file system or SQLite",
              },
            ])

      log.blank()
      await create(resolvedName, {
        framework: resolvedFw,
        runtime: resolvedRuntime,
        noInstall: opts.install === false,
      })
    },
  )

cli
  .command("dev", "Start the app in development mode")
  .option("--cwd <dir>", "Project directory", { default: process.cwd() })
  .action(async (opts: { cwd: string }) => {
    await dev({ cwd: opts.cwd })
  })

cli
  .command("build", "Build and package the app as a single distributable binary")
  .option("--cwd <dir>", "Project directory", { default: process.cwd() })
  .option("--outfile <path>", "Output binary path (default: release/<name>[.exe])")
  .action(async (opts: { cwd: string; outfile?: string }) => {
    await build(opts)
  })

cli
  .command("init", "Add tynd to an existing project")
  .option("--cwd <dir>", "Project directory", { default: process.cwd() })
  .option("-r, --runtime <r>", "Runtime: full | lite")
  .option("-f, --force", "Overwrite existing files", { default: false })
  .action(async (opts: { cwd: string; runtime?: string; force: boolean }) => {
    let runtime: "full" | "lite"
    if (opts.runtime === "full" || opts.runtime === "lite") {
      runtime = opts.runtime
    } else {
      // Interactive: ask for runtime
      const { select } = await import("./lib/prompt.ts")
      runtime = await select("Runtime", [
        { value: "full", label: "full", hint: "full — Node.js APIs" },
        { value: "lite", label: "lite", hint: "smaller binary (~10 MB), no file system or SQLite" },
      ] as const)
    }
    await init({ cwd: opts.cwd, runtime, force: opts.force })
  })

cli
  .command("clean", "Remove build artifacts")
  .option("--cwd <dir>", "Project directory", { default: process.cwd() })
  .option("-y, --yes", "Skip confirmation", { default: false })
  .option("-n, --dry-run", "Show what would be deleted without deleting", { default: false })
  .action(async (opts: { cwd: string; yes: boolean; dryRun: boolean }) => {
    await clean(opts)
  })

cli
  .command("validate", "Validate the project config and file structure")
  .option("--cwd <dir>", "Project directory", { default: process.cwd() })
  .option("--json", "Output as JSON", { default: false })
  .action(async (opts: { cwd: string; json: boolean }) => {
    await validate(opts)
  })

cli
  .command("upgrade", "Upgrade @tynd/cli and @tynd/core to latest")
  .option("-y, --yes", "Skip confirmation", { default: false })
  .action(async (opts: { yes: boolean }) => {
    await upgrade(opts)
  })

cli
  .command("info", "Show environment and project info")
  .option("--cwd <dir>", "Project directory", { default: process.cwd() })
  .option("--json", "Output as JSON", { default: false })
  .action(async (opts: { cwd: string; json: boolean }) => {
    await info(opts)
  })

cli.help()
cli.version(VERSION)

// Show help when invoked with no command
if (process.argv.slice(2).length === 0) {
  cli.outputHelp()
  process.exit(0)
}

// Parse global verbosity flags early so they take effect across all commands.
const rawArgs = process.argv.slice(2)
if (rawArgs.includes("--verbose")) setLogLevel("verbose")
else if (rawArgs.includes("--quiet")) setLogLevel("quiet")

// Typo suggestion: if the first positional isn't a known command, propose the
// closest match before cac prints its generic "Unknown command" error.
const KNOWN_COMMANDS = ["create", "dev", "build", "init", "clean", "validate", "upgrade", "info"]
const firstArg = rawArgs.find((a) => !a.startsWith("-"))
if (firstArg && !KNOWN_COMMANDS.includes(firstArg)) {
  const suggestion = closestCommand(firstArg, KNOWN_COMMANDS)
  if (suggestion) {
    process.stderr.write(`\n  error: unknown command "${firstArg}"\n`)
    process.stderr.write(`         did you mean ${log.cyan(`tynd ${suggestion}`)}?\n\n`)
    process.exit(1)
  }
}

process.on("unhandledRejection", (err) => {
  if (err instanceof ConfigError) {
    log.error("tynd.config is invalid:")
    for (const issue of err.issues) log.dim(`  • ${issue}`)
    process.exit(1)
  }
  throw err
})

try {
  cli.parse()
} catch (err: unknown) {
  if (err instanceof ConfigError) {
    log.error("tynd.config is invalid:")
    for (const issue of err.issues) log.dim(`  • ${issue}`)
    process.exit(1)
  }
  // CACError = missing required args, unknown command, etc. → show command help
  if (err instanceof Error && err.constructor.name === "CACError") {
    const subcmd = process.argv[2]
    if (subcmd) {
      process.stderr.write(`\n  error: ${err.message}\n\n`)
      process.argv.push("--help")
      cli.parse()
    } else {
      cli.outputHelp()
    }
    process.exit(1)
  }
  throw err
}

/**
 * Find the command with the smallest Levenshtein distance to `input`, but only
 * return it if the match is close enough (distance ≤ half the input length).
 */
function closestCommand(input: string, commands: string[]): string | null {
  let best: { cmd: string; dist: number } | null = null
  for (const cmd of commands) {
    const d = levenshtein(input, cmd)
    if (!best || d < best.dist) best = { cmd, dist: d }
  }
  if (!best) return null
  return best.dist <= Math.max(1, Math.floor(input.length / 2)) ? best.cmd : null
}

function levenshtein(a: string, b: string): number {
  const m = a.length
  const n = b.length
  if (m === 0) return n
  if (n === 0) return m
  const row = Array.from({ length: n + 1 }, (_, i) => i)
  for (let i = 1; i <= m; i++) {
    let prev = i
    for (let j = 1; j <= n; j++) {
      const cur = a[i - 1] === b[j - 1] ? row[j - 1]! : Math.min(row[j - 1]!, row[j]!, prev) + 1
      row[j - 1] = prev
      prev = cur
    }
    row[n] = prev
  }
  return row[n]!
}
