import path from "path";
import { loadConfig } from "../utils/config";
import type { BuildTool } from "../utils/types";

/** Server-side / full-stack frameworks incompatible with bunview. */
const SERVER_FRAMEWORKS: Record<string, string> = {
  "next":                "Next.js",
  "@remix-run/react":    "Remix",
  "@remix-run/node":     "Remix",
  "blitz":               "Blitz.js",
  "@redwoodjs/core":     "RedwoodJS",
  "nuxt":                "Nuxt",
  "@sveltejs/kit":       "SvelteKit",
  "@solidjs/start":      "SolidStart",
  "@analogjs/platform":  "Analog",
  "@tanstack/start":     "TanStack Start",
  "@tanstack/react-start": "TanStack Start",
  "astro":               "Astro",
  "gatsby":              "Gatsby",
  "@builder.io/qwik-city": "Qwik City",
  "vike":                "Vike",
  "fresh":               "Fresh",
};

function getDevConfig(buildTool: BuildTool): { url: string; command: string } | null {
  switch (buildTool) {
    case "vite":    return { url: "http://localhost:5173", command: "bunx --bun vite" };
    case "parcel":  return { url: "http://localhost:1234", command: "bunx parcel" };
    case "angular": return { url: "http://localhost:4200", command: "bunx ng serve" };
    case "rsbuild": return { url: "http://localhost:3000", command: "bunx rsbuild dev" };
    case "cra":     return { url: "http://localhost:3000", command: "bunx react-scripts start" };
    case "webpack": return { url: "http://localhost:8080", command: "bunx webpack serve" };
    default:        return null;
  }
}

/** Detect build tool from package.json dependencies (lightweight — only for dev server auto-detection). */
async function detectBuildTool(cwd: string): Promise<BuildTool> {
  const pkgPath = path.join(cwd, "package.json");
  if (!await Bun.file(pkgPath).exists()) return "none";

  const pkg = await Bun.file(pkgPath).json() as {
    dependencies?: Record<string, string>;
    devDependencies?: Record<string, string>;
  };
  const allDeps = { ...pkg.dependencies, ...pkg.devDependencies };

  // Reject server frameworks
  for (const dep of Object.keys(SERVER_FRAMEWORKS)) {
    if (allDeps[dep]) return "none";
  }

  if (allDeps["vite"] || Object.keys(allDeps).some(k => k.startsWith("@vitejs/")))  return "vite";
  if (allDeps["react-scripts"])                                                       return "cra";
  if (allDeps["@angular/cli"] || allDeps["@angular-devkit/build-angular"])            return "angular";
  if (allDeps["parcel"] || allDeps["parcel-bundler"])                                 return "parcel";
  if (allDeps["@rsbuild/core"])                                                       return "rsbuild";
  if (allDeps["webpack"] || allDeps["webpack-cli"])                                   return "webpack";

  return "none";
}

/** Auto-detect the frontend dev server from package.json dependencies. */
async function autoDetectDevServer(cwd: string): Promise<{ url: string; command: string } | null> {
  const buildTool = await detectBuildTool(cwd);
  return getDevConfig(buildTool);
}

/** Poll until a URL responds (or timeout). */
async function waitForServer(url: string, timeout = 30_000): Promise<boolean> {
  const start = Date.now();
  while (Date.now() - start < timeout) {
    try {
      const res = await fetch(url);
      if (res.ok || res.status === 404) return true;
    } catch {}
    await new Promise(r => setTimeout(r, 500));
  }
  return false;
}

import { log } from "../utils/colors";

export interface DevOptions {
  port?:         number;
  host?:         string;
  open?:         boolean;
  inspect?:      boolean;
  backendOnly?:  boolean;
  frontendOnly?: boolean;
}

export async function runDev(opts: DevOptions = {}) {
  return runApp(true, opts, []);
}

export async function runStart(extraArgs: string[] = []) {
  return runApp(false, {}, extraArgs);
}

async function runApp(watch: boolean, opts: DevOptions, extraArgs: string[]) {
  const config  = await loadConfig();
  const entry   = path.resolve(process.cwd(), config.entry);
  const env     = { ...process.env } as Record<string, string>;
  let devServerProc: ReturnType<typeof Bun.spawn> | null = null;

  if (watch && !opts.backendOnly) {
    const autoCfg = config.dev ?? await autoDetectDevServer(process.cwd());
    const devCfg = autoCfg ? overrideDevCfg(autoCfg, opts) : null;
    if (devCfg) {
      env.BUNVIEW_DEV_URL = devCfg.url;
      const [cmd, ...cmdArgs] = devCfg.command.split(" ");
      log.info(`Starting dev server: ${devCfg.command}`);
      devServerProc = Bun.spawn([cmd, ...cmdArgs], {
        stdout: "inherit", stderr: "inherit", env, cwd: process.cwd(),
      });

      const ready = await waitForServer(devCfg.url);
      if (!ready) {
        log.error(`Dev server did not start at ${devCfg.url} within 30s`);
        devServerProc.kill();
        process.exit(1);
      }
      log.ok(`Dev server ready → ${devCfg.url} (HMR enabled)`);
      if (opts.open) {
        try {
          const openCmd = process.platform === "win32" ? ["cmd", "/c", "start", "", devCfg.url]
                        : process.platform === "darwin" ? ["open", devCfg.url]
                        : ["xdg-open", devCfg.url];
          Bun.spawn(openCmd, { stdout: "ignore", stderr: "ignore" });
        } catch { /* best-effort */ }
      }
    }
  }

  if (opts.frontendOnly) {
    // Nothing more to do — dev server is already running, exit waits for Ctrl+C.
    log.info("Frontend-only mode — backend not started.");
    process.on("SIGINT", () => { devServerProc?.kill(); process.exit(0); });
    await new Promise(() => {});  // block
    return;
  }

  const bunArgs = watch ? ["bun", "--watch", entry, ...extraArgs] : ["bun", entry, ...extraArgs];
  if (opts.inspect) bunArgs.splice(1, 0, "--inspect");

  const proc = Bun.spawn(bunArgs, {
    stdout: "inherit", stderr: "inherit", stdin: "inherit", env,
  });

  const cleanup = () => { proc.kill(); devServerProc?.kill(); };
  process.on("SIGINT", cleanup);
  process.on("SIGTERM", cleanup);

  const code = await proc.exited;
  devServerProc?.kill();
  process.exit(code);
}

/** Override detected dev server URL / command with user-provided flags. */
function overrideDevCfg(
  cfg: { url: string; command: string },
  opts: DevOptions,
): { url: string; command: string } {
  const u = new URL(cfg.url);
  if (opts.host) u.hostname = opts.host;
  if (opts.port) u.port = String(opts.port);
  let command = cfg.command;
  if (opts.port) command += ` --port ${opts.port}`;
  if (opts.host) command += ` --host ${opts.host}`;
  return { url: u.toString().replace(/\/$/, ""), command };
}
