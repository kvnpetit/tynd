import { existsSync, readFileSync } from "node:fs"
import { mkdir, writeFile } from "node:fs/promises"
import path from "node:path"
import { detectFrontend } from "../lib/detect.ts"
import { log } from "../lib/logger.ts"
import { loadPackageJson, type PackageJson } from "../lib/pkg.ts"
import { backendMain, tyndConfig } from "../lib/template.ts"
import { VERSION } from "../lib/version.ts"

export interface InitOptions {
  cwd: string
  runtime: "full" | "lite"
  force: boolean
}

export async function init(opts: InitOptions): Promise<void> {
  const configPath = path.join(opts.cwd, "tynd.config.ts")

  log.blank()

  if (existsSync(configPath) && !opts.force) {
    log.warn("tynd.config.ts already exists. Use --force to overwrite.")
    log.blank()
    return
  }

  let name = path.basename(opts.cwd)
  const pkgPath = path.join(opts.cwd, "package.json")
  let pkg: PackageJson | null = await loadPackageJson(opts.cwd)

  if (pkg) {
    if (pkg.name) name = pkg.name
  } else {
    // No package.json -> synthesize a minimal one so `bun install` and the
    // generated deps (@tynd/core, @tynd/host) have somewhere to land.
    pkg = { name, version: "0.0.0" }
    log.step(`${log.cyan("create")}  package.json`)
  }

  const frontend = await detectFrontend(opts.cwd)

  if (frontend.blockedBy) {
    log.error(`${frontend.blockedBy} detected — incompatible with server-side frameworks.`)
    log.dim("  Requires a pure SPA (React, Vue, Svelte, Angular, Solid, Lit, Preact…)")
    log.blank()
    process.exit(1)
  }

  const hasFrontend = frontend.buildTool !== "none"

  log.info(`Initializing in ${log.cyan(name)}`)
  if (hasFrontend) {
    log.step(
      `Detected: ${log.cyan(frontend.buildTool)} -> output dir: ${log.gray(frontend.outDir)}`,
    )
  }
  log.blank()

  const frontendDir = hasFrontend ? frontend.outDir : "frontend"
  // Without a framework, Bun.build turns frontend/main.ts -> frontend/main.js
  // on dev/build. The generated HTML references the produced .js file.
  const frontendEntry = hasFrontend ? undefined : "frontend/main.ts"

  await write(
    opts.cwd,
    "tynd.config.ts",
    tyndConfig(name, opts.runtime, frontendDir, frontendEntry),
    opts.force,
  )

  const backendRelFront = hasFrontend
    ? `/../${frontendDir}` // e.g. /../dist
    : "/../frontend"

  await write(
    opts.cwd,
    "backend/main.ts",
    backendMain(name, opts.runtime, backendRelFront),
    opts.force,
  )

  if (!hasFrontend) {
    await write(
      opts.cwd,
      "frontend/index.html",
      `<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8" />
  <title>${name}</title>
  <script type="module" src="./main.js"></script>
</head>
<body>
  <h1>${name}</h1>
  <p id="msg">Loading…</p>
</body>
</html>
`,
      opts.force,
    )

    await write(
      opts.cwd,
      "frontend/main.ts",
      `import { createBackend } from "@tynd/core/client"
import type * as backend from "../backend/main"

const api = createBackend<typeof backend>()

api.on("ready", ({ message }) => {
  document.getElementById("msg")!.textContent = message
})
`,
      opts.force,
    )
  }

  if (pkg) {
    await patchPackageJson(pkgPath, pkg)
  }

  await patchGitignore(opts.cwd)

  log.blank()
  log.success("Done!")
  log.blank()

  if (hasFrontend) {
    log.dim(`  ${log.cyan("tynd dev")}   — starts ${frontend.buildTool} dev server + tynd host`)
    log.dim(
      `  ${log.cyan("tynd start")} — classic JS build of frontend + backend, then run (no HMR)`,
    )
    log.dim(`  ${log.cyan("tynd build")} — runs ${frontend.buildTool} build + packages .exe`)
  } else {
    log.dim(`  ${log.cyan("tynd dev")}   — starts tynd host`)
    log.dim(`  ${log.cyan("tynd start")} — classic JS build, then run (no HMR)`)
    log.dim(`  ${log.cyan("tynd build")} — bundles + packages .exe`)
  }

  log.blank()
}

async function write(cwd: string, rel: string, content: string, force: boolean): Promise<void> {
  const abs = path.join(cwd, rel)
  if (existsSync(abs) && !force) {
    log.step(`${log.gray("skip")}    ${rel} (already exists)`)
    return
  }
  await mkdir(path.dirname(abs), { recursive: true })
  await writeFile(abs, content, "utf-8")
  log.step(`${log.cyan("create")}  ${rel}`)
}

async function patchGitignore(cwd: string): Promise<void> {
  const gitignorePath = path.join(cwd, ".gitignore")
  const entry = ".tynd/"
  if (existsSync(gitignorePath)) {
    const content = readFileSync(gitignorePath, "utf-8")
    if (content.includes(entry)) return
    const newContent = content.endsWith("\n") ? `${content + entry}\n` : `${content}\n${entry}\n`
    await writeFile(gitignorePath, newContent, "utf-8")
    log.step(`${log.cyan("patch")}   .gitignore (added ${entry})`)
  }
  // No .gitignore -> silently skip; Vite scaffold always creates one
}

async function patchPackageJson(pkgPath: string, pkg: PackageJson): Promise<void> {
  const scripts = pkg.scripts ?? {}

  // Preserve existing scripts under `build:ui` / `dev:ui` if we'd overwrite them
  if (scripts["build"] && scripts["build"] !== "tynd build") {
    scripts["build:ui"] = scripts["build"]
  }
  if (scripts["dev"] && scripts["dev"] !== "tynd dev") {
    scripts["dev:ui"] = scripts["dev"]
  }
  scripts["dev"] = "tynd dev"
  scripts["start"] = "tynd start"
  scripts["build"] = "tynd build"
  pkg.scripts = scripts

  // Pin runtime deps to the CLI version so they never outrun the CLI.
  const deps = pkg.dependencies ?? {}
  const range = `^${VERSION}`
  if (!deps["@tynd/core"]) deps["@tynd/core"] = range
  if (!deps["@tynd/host"]) deps["@tynd/host"] = range
  pkg.dependencies = deps

  await writeFile(pkgPath, `${JSON.stringify(pkg, null, 2)}\n`, "utf-8")
  log.step(`${log.cyan("patch")}   package.json`)
  log.debug(`patchPackageJson: deps @tynd/core, @tynd/host -> ${range}`)
}
