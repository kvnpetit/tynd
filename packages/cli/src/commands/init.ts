import { existsSync, readFileSync } from "node:fs"
import { mkdir, writeFile } from "node:fs/promises"
import path from "node:path"
import { detectFrontend } from "../lib/detect.ts"
import { log } from "../lib/logger.ts"
import { backendMain, vornConfig } from "../lib/template.ts"
import { VERSION } from "../lib/version.ts"

export interface InitOptions {
  cwd: string
  runtime: "full" | "lite"
  force: boolean
}

export async function init(opts: InitOptions): Promise<void> {
  const configPath = path.join(opts.cwd, "vorn.config.ts")

  log.blank()

  if (existsSync(configPath) && !opts.force) {
    log.warn("vorn.config.ts already exists. Use --force to overwrite.")
    log.blank()
    return
  }

  let name = path.basename(opts.cwd)
  const pkgPath = path.join(opts.cwd, "package.json")
  let pkg: Record<string, unknown> | null = null

  if (existsSync(pkgPath)) {
    try {
      pkg = (await Bun.file(pkgPath).json()) as Record<string, unknown>
      if (typeof pkg["name"] === "string") name = pkg["name"]
    } catch {
      /* ignore */
    }
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
    log.step(`Detected: ${log.cyan(frontend.buildTool)} → output dir: ${log.gray(frontend.outDir)}`)
  }
  log.blank()

  const frontendDir = hasFrontend ? frontend.outDir : "frontend"
  // Without a framework, Bun.build turns frontend/main.ts → frontend/main.js
  // on dev/build. The generated HTML references the produced .js file.
  const frontendEntry = hasFrontend ? undefined : "frontend/main.ts"

  await write(
    opts.cwd,
    "vorn.config.ts",
    vornConfig(name, opts.runtime, frontendDir, frontendEntry),
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
      `import { createBackend } from "@vorn/core/client"
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
    log.dim(`  ${log.cyan("vorn dev")}   — starts ${frontend.buildTool} dev server + vorn host`)
    log.dim(`  ${log.cyan("vorn build")} — runs ${frontend.buildTool} build + bundles backend`)
  } else {
    log.dim(`  ${log.cyan("vorn dev")}   — starts vorn host`)
    log.dim(`  ${log.cyan("vorn build")} — bundles backend`)
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
  const entry = ".vorn/"
  if (existsSync(gitignorePath)) {
    const content = readFileSync(gitignorePath, "utf-8")
    if (content.includes(entry)) return
    const newContent = content.endsWith("\n") ? `${content + entry}\n` : `${content}\n${entry}\n`
    await writeFile(gitignorePath, newContent, "utf-8")
    log.step(`${log.cyan("patch")}   .gitignore (added ${entry})`)
  }
  // No .gitignore → silently skip; Vite scaffold always creates one
}

async function patchPackageJson(pkgPath: string, pkg: Record<string, unknown>): Promise<void> {
  const scripts = (pkg["scripts"] as Record<string, string> | undefined) ?? {}

  // Preserve existing scripts under a different name if they'd be overwritten
  if (scripts["build"] && scripts["build"] !== "vorn build") {
    scripts["build:ui"] = scripts["build"]
  }
  if (scripts["dev"] && scripts["dev"] !== "vorn dev") {
    scripts["dev:ui"] = scripts["dev"]
  }

  scripts["dev"] = "vorn dev"
  scripts["build"] = "vorn build"
  pkg["scripts"] = scripts

  // Pin to the CLI's own version so `vorn dev` / `vorn build` never pull a
  // newer runtime than the CLI was built against. Caret lets users receive
  // backwards-compatible patch updates via `bun install`.
  const deps = (pkg["dependencies"] as Record<string, string> | undefined) ?? {}
  const range = `^${VERSION}`
  if (!deps["@vorn/core"]) deps["@vorn/core"] = range
  if (!deps["@vorn/host"]) deps["@vorn/host"] = range
  pkg["dependencies"] = deps

  await writeFile(pkgPath, `${JSON.stringify(pkg, null, 2)}\n`, "utf-8")
  log.step(
    `${log.cyan("patch")}   package.json (scripts: dev, build — deps: @vorn/core, @vorn/host)`,
  )
}
