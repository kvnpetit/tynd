import { existsSync, rmSync } from "node:fs"
import path from "node:path"
import { exec } from "../lib/exec.ts"
import { log } from "../lib/logger.ts"
import { confirm, withSpinner } from "../lib/prompt.ts"
import { init } from "./init.ts"

export const FRAMEWORKS = [
  { value: "react", label: "React", hint: "Vite + React + TypeScript", viteTemplate: "react-ts" },
  { value: "vue", label: "Vue", hint: "Vite + Vue 3 + TypeScript", viteTemplate: "vue-ts" },
  {
    value: "svelte",
    label: "Svelte",
    hint: "Vite + Svelte + TypeScript",
    viteTemplate: "svelte-ts",
  },
  { value: "solid", label: "Solid", hint: "Vite + Solid + TypeScript", viteTemplate: "solid-ts" },
  {
    value: "preact",
    label: "Preact",
    hint: "Vite + Preact + TypeScript",
    viteTemplate: "preact-ts",
  },
  { value: "lit", label: "Lit", hint: "Vite + Lit + TypeScript", viteTemplate: "lit-ts" },
  { value: "angular", label: "Angular", hint: "Angular CLI + TypeScript", viteTemplate: null },
] as const

export type Framework = (typeof FRAMEWORKS)[number]["value"]

export interface CreateOptions {
  framework: Framework
  runtime: "full" | "lite"
  /** Skip the install prompt and never run `bun install`. Default: ask. */
  noInstall?: boolean
}

export async function create(name: string, opts: CreateOptions): Promise<void> {
  const projectDir = path.resolve(process.cwd(), name)
  const fw = FRAMEWORKS.find((f) => f.value === opts.framework)
  if (!fw) {
    log.error(`Unknown framework: "${opts.framework}"`)
    process.exit(1)
  }

  if (existsSync(projectDir)) {
    log.error(`Directory "${name}" already exists.`)
    process.exit(1)
  }

  log.blank()
  log.info(`Creating ${log.bold(name)} — ${log.cyan(fw.label)} / ${log.cyan(opts.runtime)}`)
  log.blank()

  try {
    if (fw.viteTemplate !== null) {
      await withSpinner(`Scaffolding ${fw.label} project with Vite`, () =>
        exec("bun", ["create", "vite@latest", name, "--template", fw.viteTemplate], {
          cwd: process.cwd(),
          silent: true,
        }),
      )
    } else {
      await withSpinner("Scaffolding Angular project", () =>
        exec(
          "bunx",
          [
            "@angular/cli@latest",
            "new",
            name,
            "--defaults",
            "--skip-git",
            "--skip-install",
            "--ssr=false",
          ],
          { cwd: process.cwd(), silent: true },
        ),
      )
    }
  } catch (err) {
    log.hint(
      `Scaffolding failed: ${err instanceof Error ? err.message : String(err)}`,
      "Check your network connection and that bunx can reach the npm registry.",
    )
    // Remove the partially-created project so a retry starts from a clean slate.
    if (existsSync(projectDir)) {
      try {
        rmSync(projectDir, { recursive: true, force: true })
      } catch {
        /* best-effort cleanup */
      }
    }
    process.exit(1)
  }

  log.blank()

  await init({ cwd: projectDir, runtime: opts.runtime, force: false })

  const shouldInstall = opts.noInstall ? false : await confirm("Install dependencies now?", true)

  if (shouldInstall) {
    try {
      await withSpinner("Installing dependencies", () =>
        exec("bun", ["install"], { cwd: projectDir, silent: true }),
      )
    } catch (err) {
      log.warn(`bun install failed: ${err}`)
    }
  }

  log.blank()
  log.success(`${log.bold(name)} is ready!`)
  log.blank()
  log.dim(`  cd ${name}`)
  if (!shouldInstall) log.dim(`  bun install`)
  log.dim(`  vorn dev`)
  log.blank()
}
