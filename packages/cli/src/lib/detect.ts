import { existsSync } from "node:fs"
import path from "node:path"
import { log } from "./logger.ts"
import { allDeps, loadPackageJson } from "./pkg.ts"

export type Platform = "windows" | "macos" | "linux"
export type Arch = "x64" | "arm64"
export type BuildTool = "vite" | "cra" | "angular" | "parcel" | "rsbuild" | "webpack" | "none"

export interface FrontendInfo {
  buildTool: BuildTool
  /** Production build output dir (relative to cwd) */
  outDir: string
  /** Command to run a production build (used by `tynd build`) */
  buildCommand: string | null
  /** Command to start the dev server */
  devCommand: string | null
  /** Default URL the dev server listens on */
  devUrl: string | null
  /** Set when a server-side framework was detected — incompatible */
  blockedBy?: string
}

export function getPlatform(): Platform {
  switch (process.platform) {
    case "win32":
      return "windows"
    case "darwin":
      return "macos"
    default:
      return "linux"
  }
}

export function getArch(): Arch {
  return process.arch === "arm64" ? "arm64" : "x64"
}

export function binaryName(runtime: "full" | "lite"): string {
  const suffix = getPlatform() === "windows" ? ".exe" : ""
  return `tynd-${runtime}${suffix}`
}

/**
 * Locate tynd-full / tynd-lite binary.
 * Search order: Cargo workspace target/release → target/debug → node_modules/@tynd/{runtime}/bin → PATH
 */
export function findBinary(runtime: "full" | "lite", cwd: string): string | null {
  const name = binaryName(runtime)
  const plat = getPlatform()
  const arch = getArch()
  log.debug(`findBinary(${runtime}): searching for ${name} on ${plat}-${arch}`)

  // 1. Cargo workspace root (walk up from cwd) — prefer release over debug
  let dir = cwd
  for (let i = 0; i < 6; i++) {
    const release = path.join(dir, "target", "release", name)
    const debug = path.join(dir, "target", "debug", name)
    if (existsSync(release)) {
      log.debug(`findBinary: matched workspace release → ${release}`)
      return release
    }
    if (existsSync(debug)) {
      log.debug(`findBinary: matched workspace debug → ${debug}`)
      return debug
    }
    const parent = path.dirname(dir)
    if (parent === dir) break
    dir = parent
  }

  // 2. Published npm package (@tynd/host contains both runtimes)
  const nmBin = path.join(cwd, "node_modules", "@tynd/host", "bin", `${plat}-${arch}`, name)
  if (existsSync(nmBin)) {
    log.debug(`findBinary: matched node_modules → ${nmBin}`)
    return nmBin
  }

  // 3. System PATH
  const which = process.platform === "win32" ? "where" : "which"
  try {
    const result = Bun.spawnSync([which, name], { stdout: "pipe", stderr: "pipe" })
    if (result.exitCode === 0) {
      const p = new TextDecoder().decode(result.stdout).trim().split("\n")[0]!.trim()
      if (p) {
        log.debug(`findBinary: matched PATH → ${p}`)
        return p
      }
    }
  } catch {
    /* not found */
  }

  log.debug(`findBinary(${runtime}): no match found`)
  return null
}

/** Server-side frameworks incompatible with tynd (they own the server). */
const SERVER_FRAMEWORKS: Record<string, string> = {
  next: "Next.js",
  "@remix-run/react": "Remix",
  "@remix-run/node": "Remix",
  "@remix-run/cloudflare": "Remix",
  "@remix-run/deno": "Remix",
  gatsby: "Gatsby",
  "@redwoodjs/core": "RedwoodJS",
  blitz: "Blitz.js",
  nuxt: "Nuxt",
  "@nuxtjs/bridge": "Nuxt Bridge",
  "@sveltejs/kit": "SvelteKit",
  "@solidjs/start": "SolidStart",
  "@angular/platform-server": "Angular Universal",
  "@nguniversal/express-engine": "Angular Universal",
  "@analogjs/core": "Analog",
  "@analogjs/platform": "Analog",
  "@builder.io/qwik-city": "Qwik City",
  astro: "Astro",
  "@tanstack/start": "TanStack Start",
  "@tanstack/react-start": "TanStack Start",
  vike: "Vike (vite-plugin-ssr)",
  "vite-plugin-ssr": "vite-plugin-ssr",
}

const TOOL_COMMANDS: Record<
  BuildTool,
  { devCommand: string; devUrl: string; buildCommand: string } | null
> = {
  vite: {
    devCommand: "bunx --bun vite",
    devUrl: "http://localhost:5173",
    buildCommand: "bunx --bun vite build",
  },
  cra: {
    devCommand: "bunx react-scripts start",
    devUrl: "http://localhost:3000",
    buildCommand: "bunx react-scripts build",
  },
  angular: {
    devCommand: "bunx ng serve",
    devUrl: "http://localhost:4200",
    buildCommand: "bunx ng build",
  },
  parcel: {
    devCommand: "bunx parcel",
    devUrl: "http://localhost:1234",
    buildCommand: "bunx parcel build",
  },
  rsbuild: {
    devCommand: "bunx rsbuild dev",
    devUrl: "http://localhost:3000",
    buildCommand: "bunx rsbuild build",
  },
  webpack: {
    devCommand: "bunx webpack serve",
    devUrl: "http://localhost:8080",
    buildCommand: "bunx webpack --mode production",
  },
  none: null,
}

const DEFAULT_OUT_DIR: Record<BuildTool, string> = {
  vite: "dist",
  cra: "build",
  angular: "dist",
  parcel: "dist",
  rsbuild: "dist",
  webpack: "dist",
  none: "frontend",
}

/** Read the actual output dir from the framework's config file. */
async function resolveOutDir(cwd: string, tool: BuildTool): Promise<string> {
  if (tool === "vite") {
    for (const name of ["vite.config.ts", "vite.config.js", "vite.config.mts", "vite.config.mjs"]) {
      const f = path.join(cwd, name)
      if (!existsSync(f)) continue
      const content = await Bun.file(f).text()
      const m = content.match(/outDir\s*:\s*['"]([^'"]+)['"]/)
      if (m) return m[1]!
    }
  }
  if (tool === "angular") {
    const f = path.join(cwd, "angular.json")
    if (existsSync(f)) {
      try {
        const json = JSON.parse(await Bun.file(f).text()) as Record<string, unknown>
        const projects = json["projects"] as Record<string, unknown> | undefined
        if (projects) {
          // Prefer the first project with projectType "application" over a library
          const projectName =
            Object.keys(projects).find(
              (k) => (projects[k] as Record<string, unknown>)?.["projectType"] === "application",
            ) ?? Object.keys(projects)[0]
          if (projectName) {
            const proj = projects[projectName] as Record<string, unknown>
            const build = (proj?.["architect"] as Record<string, unknown>)?.["build"] as Record<
              string,
              unknown
            >
            const builder = build?.["builder"] as string | undefined
            const outPathRaw = (build?.["options"] as Record<string, unknown>)?.["outputPath"]
            // Angular CLI defaults to `dist/<project-name>` when outputPath is omitted.
            const outPath = typeof outPathRaw === "string" ? outPathRaw : `dist/${projectName}`
            // Angular 17+ (@angular/build:application) puts browser assets in <outputPath>/browser/
            if (builder?.includes("@angular/build:application")) {
              return `${outPath}/browser`
            }
            return outPath
          }
        }
      } catch {
        /* fallback */
      }
      // Regex fallback
      const content = await Bun.file(f).text()
      const m = content.match(/"outputPath"\s*:\s*"([^"]+)"/)
      if (m) return m[1]!
    }
  }
  if (tool === "rsbuild") {
    for (const name of ["rsbuild.config.ts", "rsbuild.config.js"]) {
      const f = path.join(cwd, name)
      if (!existsSync(f)) continue
      const content = await Bun.file(f).text()
      const m = content.match(/distPath\s*:\s*\{[^}]*root\s*:\s*['"]([^'"]+)['"]/s)
      if (m) return m[1]!
    }
  }
  return DEFAULT_OUT_DIR[tool]
}

/** Detect the frontend framework from package.json dependencies. */
export async function detectFrontend(cwd: string): Promise<FrontendInfo> {
  const pkg = await loadPackageJson(cwd)
  if (!pkg) {
    return {
      buildTool: "none",
      outDir: "frontend",
      buildCommand: null,
      devCommand: null,
      devUrl: null,
    }
  }
  const deps = allDeps(pkg)

  // Block incompatible server frameworks
  for (const [dep, name] of Object.entries(SERVER_FRAMEWORKS)) {
    if (deps[dep]) {
      return {
        buildTool: "none",
        outDir: "frontend",
        buildCommand: null,
        devCommand: null,
        devUrl: null,
        blockedBy: name,
      }
    }
  }

  let tool: BuildTool = "none"
  if (deps["vite"] || Object.keys(deps).some((k) => k.startsWith("@vitejs/"))) tool = "vite"
  else if (deps["react-scripts"]) tool = "cra"
  else if (deps["@angular/cli"] || deps["@angular-devkit/build-angular"]) tool = "angular"
  else if (deps["parcel"] || deps["parcel-bundler"]) tool = "parcel"
  else if (deps["@rsbuild/core"]) tool = "rsbuild"
  else if (deps["webpack"] || deps["webpack-cli"]) tool = "webpack"
  log.debug(`detectFrontend: ${tool}`)

  const outDir = await resolveOutDir(cwd, tool)
  log.debug(`detectFrontend: outDir=${outDir}`)
  const cmds = TOOL_COMMANDS[tool]

  return {
    buildTool: tool,
    outDir,
    buildCommand: cmds?.buildCommand ?? null,
    devCommand: cmds?.devCommand ?? null,
    devUrl: cmds?.devUrl ?? null,
  }
}
