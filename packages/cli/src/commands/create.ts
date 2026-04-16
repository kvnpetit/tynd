import { existsSync } from "node:fs"
import path from "node:path"
import { exec } from "../lib/exec.ts"
import { log } from "../lib/logger.ts"
import { confirm } from "../lib/prompt.ts"
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

  if (fw.viteTemplate !== null) {
    log.step(`Scaffolding with ${log.cyan("Vite")} (${fw.hint})…`)
    await exec("bun", ["create", "vite@latest", name, "--template", fw.viteTemplate], {
      cwd: process.cwd(),
    })
    log.success(`${fw.label} project created`)
  } else {
    log.step(`Scaffolding with ${log.cyan("Angular CLI")}…`)
    await exec(
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
      { cwd: process.cwd() },
    )
    log.success("Angular project created")
  }

  log.blank()

  await init({ cwd: projectDir, runtime: opts.runtime, force: false })

  const shouldInstall = opts.noInstall ? false : await confirm("Install dependencies now?", true)

  if (shouldInstall) {
    log.step("Installing dependencies…")
    try {
      await exec("bun", ["install"], { cwd: projectDir })
      log.success("Dependencies installed")
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
