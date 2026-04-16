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
import { log } from "./lib/logger.ts"
import { VERSION } from "./lib/version.ts"

const cli = cac("vorn")

const frameworkValues = FRAMEWORKS.map((f) => f.value).join(" | ")

cli
  .command("create [name]", "Scaffold a new Vorn project")
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
          log.info("Create a new Vorn project")
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
  .command("init", "Add vorn to an existing project")
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
  .action(async (opts: { cwd: string; yes: boolean }) => {
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
  .command("upgrade", "Upgrade @vorn/cli and @vorn/core to latest")
  .option("-y, --yes", "Skip confirmation", { default: false })
  .action(async (opts: { yes: boolean }) => {
    await upgrade(opts)
  })

cli.command("info", "Show environment and project info").action(async () => {
  await info()
})

cli.help()
cli.version(VERSION)

// Show help when invoked with no command
if (process.argv.slice(2).length === 0) {
  cli.outputHelp()
  process.exit(0)
}

try {
  cli.parse()
} catch (err: unknown) {
  // CACError = missing required args, unknown command, etc. → show command help
  if (err instanceof Error && err.constructor.name === "CACError") {
    const subcmd = process.argv[2]
    if (subcmd) {
      process.stderr.write(`\n  error: ${err.message}\n\n`)
      // Re-parse with --help to show that command's help
      process.argv.push("--help")
      cli.parse()
    } else {
      cli.outputHelp()
    }
    process.exit(1)
  }
  throw err
}
