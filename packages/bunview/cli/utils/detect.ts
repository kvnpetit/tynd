import path from "path";
import type { BuildTool, UILibrary, DetectedProject } from "./types";

export const BUILD_TOOL_LABELS: Record<BuildTool, string> = {
  vite: "Vite", cra: "Create React App", angular: "Angular CLI",
  parcel: "Parcel", rsbuild: "Rsbuild", webpack: "Webpack", none: "",
};

export const UI_LABELS: Record<UILibrary, string> = {
  react: "React", vue: "Vue", svelte: "Svelte", solid: "Solid",
  preact: "Preact", angular: "Angular", lit: "Lit", qwik: "Qwik", vanilla: "Vanilla",
};

/** Default build output directory for each build tool. */
export const DEFAULT_OUT_DIR: Record<BuildTool, string> = {
  vite: "dist", cra: "build", angular: "dist", parcel: "dist",
  rsbuild: "dist", webpack: "dist", none: "frontend",
};

/** Server-side / full-stack frameworks incompatible with bunview. */
export const SERVER_FRAMEWORKS: Record<string, string> = {
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

export function projectLabel(p: DetectedProject): string {
  if (p.buildTool === "none") return "none";
  const tool = BUILD_TOOL_LABELS[p.buildTool];
  const ui   = p.ui !== "vanilla" && p.ui !== "angular" ? UI_LABELS[p.ui] : "";
  return ui ? `${tool} + ${ui}` : tool;
}

/** Read the framework's config to find its actual build output dir. */
export async function resolveOutDir(dir: string, tool: BuildTool): Promise<string> {
  const fallback = DEFAULT_OUT_DIR[tool] ?? "dist";

  if (tool === "vite") {
    for (const name of ["vite.config.ts", "vite.config.js", "vite.config.mts", "vite.config.mjs"]) {
      const f = path.join(dir, name);
      if (!await Bun.file(f).exists()) continue;
      const content = await Bun.file(f).text();
      const m = content.match(/outDir\s*:\s*['"]([^'"]+)['"]/);
      if (m) return m[1];
    }
    return "dist";
  }

  if (tool === "angular") {
    const f = path.join(dir, "angular.json");
    if (await Bun.file(f).exists()) {
      const content = await Bun.file(f).text();
      const m = content.match(/"outputPath"\s*:\s*"([^"]+)"/);
      if (m) return m[1];
    }
    return "dist";
  }

  if (tool === "rsbuild") {
    for (const name of ["rsbuild.config.ts", "rsbuild.config.js"]) {
      const f = path.join(dir, name);
      if (!await Bun.file(f).exists()) continue;
      const content = await Bun.file(f).text();
      const m = content.match(/distPath\s*:\s*\{[^}]*root\s*:\s*['"]([^'"]+)['"]/s);
      if (m) return m[1];
    }
    return "dist";
  }

  return fallback;
}

export async function detectProject(dir: string): Promise<DetectedProject> {
  const name = path.basename(dir);
  const pkgPath = path.join(dir, "package.json");
  const hasPackageJson = await Bun.file(pkgPath).exists();
  const hasBunviewConfig = await Bun.file(path.join(dir, "bunview.config.ts")).exists()
    || await Bun.file(path.join(dir, "bunview.config.js")).exists();

  const empty: DetectedProject = {
    buildTool: "none", ui: "vanilla", outDir: "frontend", name,
    hasPackageJson, hasBunviewConfig, serverFramework: null,
  };
  if (!hasPackageJson) return empty;

  const pkg = await Bun.file(pkgPath).json() as {
    name?: string;
    dependencies?: Record<string, string>;
    devDependencies?: Record<string, string>;
  };
  const allDeps = { ...pkg.dependencies, ...pkg.devDependencies };

  for (const [dep, label] of Object.entries(SERVER_FRAMEWORKS)) {
    if (allDeps[dep]) return { ...empty, name: pkg.name ?? name, hasPackageJson, serverFramework: label };
  }

  let ui: UILibrary = "vanilla";
  if (allDeps["react"] || allDeps["react-dom"])   ui = "react";
  else if (allDeps["preact"])                     ui = "preact";
  else if (allDeps["vue"])                        ui = "vue";
  else if (allDeps["svelte"])                     ui = "svelte";
  else if (allDeps["solid-js"])                   ui = "solid";
  else if (allDeps["lit"])                        ui = "lit";
  else if (allDeps["@builder.io/qwik"])           ui = "qwik";
  else if (allDeps["@angular/core"])              ui = "angular";

  let buildTool: BuildTool = "none";
  if (allDeps["vite"] || Object.keys(allDeps).some(k => k.startsWith("@vitejs/")))  buildTool = "vite";
  else if (allDeps["react-scripts"])                                                 buildTool = "cra";
  else if (allDeps["@angular/cli"] || allDeps["@angular-devkit/build-angular"])      buildTool = "angular";
  else if (allDeps["parcel"] || allDeps["parcel-bundler"])                           buildTool = "parcel";
  else if (allDeps["@rsbuild/core"])                                                 buildTool = "rsbuild";
  else if (allDeps["webpack"] || allDeps["webpack-cli"])                             buildTool = "webpack";

  const outDir = await resolveOutDir(dir, buildTool);

  return { buildTool, ui, outDir, name: pkg.name ?? name, hasPackageJson, hasBunviewConfig, serverFramework: null };
}
