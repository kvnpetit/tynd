import { existsSync } from "node:fs"
import path from "node:path"

export type Runtime = "full" | "lite"

export interface VornConfig {
  /** "full" (Bun subprocess) or "lite" (QuickJS embedded). Default: "full" */
  runtime: Runtime
  /** Path to the backend entry file */
  backend: string
  /** Path to the production frontend directory (Vite's dist/, CRA's build/, etc.) */
  frontendDir: string
  /**
   * Frontend TypeScript/JavaScript entry point for simple projects without a
   * framework (no Vite, no CRA). Vorn builds it automatically before dev/build.
   * Ignored when a frontend framework is detected in package.json.
   */
  frontendEntry?: string
  /**
   * Override the frontend dev server URL.
   * Auto-detected from package.json (Vite → http://localhost:5173, etc.) if omitted.
   */
  devUrl?: string
  /**
   * Override the command used to start the frontend dev server.
   * Auto-detected from package.json if omitted.
   */
  devCommand?: string
  /**
   * Path to the app icon (PNG or ICO). Defaults to public/favicon.ico then public/favicon.png.
   * Used for the window title bar icon, taskbar icon, and Windows .exe icon.
   */
  icon?: string
  /** Extra args passed to the vorn-full / vorn-lite binary */
  binaryArgs?: string[]
  window?: {
    title?: string
    width?: number
    height?: number
    center?: boolean
  }
}

const DEFAULTS: VornConfig = {
  runtime: "full",
  backend: "backend/main.ts",
  frontendDir: "frontend",
}

/** Load vorn.config.ts from the project directory (defaults to cwd). */
export async function loadConfig(cwd = process.cwd()): Promise<VornConfig> {
  const candidates = [path.resolve(cwd, "vorn.config.ts"), path.resolve(cwd, "vorn.config.js")]

  for (const file of candidates) {
    if (existsSync(file)) {
      const mod = (await import(file)) as { default?: Partial<VornConfig> }
      return { ...DEFAULTS, ...mod.default }
    }
  }

  // No config file — use defaults (works for projects that follow conventions)
  return { ...DEFAULTS }
}

/** Resolve config paths relative to cwd. */
export function resolvePaths(cfg: VornConfig, cwd: string): VornConfig {
  return {
    ...cfg,
    backend: path.resolve(cwd, cfg.backend),
    frontendDir: path.resolve(cwd, cfg.frontendDir),
  }
}
