import { mkdirSync } from "fs";
import path from "path";
import { detectProject, projectLabel } from "../utils/detect";
import {
  templateConfig, templateBackend, templateBackendVanilla, templateCommands, templateInternal,
  templateBackendTsconfig, templateHtml, templateAppJs, templatePackageJson,
} from "../utils/templates";

async function writeFile(dir: string, rel: string, content: string, dryRun = false) {
  if (dryRun) {
    console.log(`  [would create] ${rel}`);
    return;
  }
  const filePath = path.join(dir, rel);
  mkdirSync(path.dirname(filePath), { recursive: true });
  await Bun.write(filePath, content);
  console.log(`  + ${rel}`);
}

/** Strip JSONC comments (line and block) so plain JSON.parse can read it. */
function stripJsonComments(src: string): string {
  return src
    .replace(/\/\*[\s\S]*?\*\//g, "")       // block comments
    .replace(/(^|[^:])\/\/[^\n]*/g, "$1");  // line comments (avoid matching // in URL strings)
}

/**
 * Patch the frontend tsconfig so imports from backend (which pull Bun/Node types
 * through `import type { AppCommands }`) don't break tsc:
 * 1. Add `backend` to `exclude` (prevents type-checking backend as program roots)
 * 2. Add `bun-types` and `node` to `compilerOptions.types` (resolves transitive Node symbols)
 * 3. Disable `erasableSyntaxOnly` (bunview uses class syntax that isn't pure erasure)
 */
async function patchAppTsconfig(dir: string): Promise<void> {
  const candidates = ["tsconfig.app.json", "tsconfig.json"];
  for (const name of candidates) {
    const p = path.join(dir, name);
    if (!await Bun.file(p).exists()) continue;
    try {
      const raw = await Bun.file(p).text();
      const obj = JSON.parse(stripJsonComments(raw)) as {
        exclude?: string[];
        include?: string[];
        compilerOptions?: Record<string, unknown>;
      };
      if (!obj.include && name === "tsconfig.json") continue;  // root project-refs file

      let changed = false;
      if (!obj.exclude?.includes("backend")) {
        obj.exclude = [...(obj.exclude ?? []), "backend"];
        changed = true;
      }
      obj.compilerOptions = obj.compilerOptions ?? {};
      const types = (obj.compilerOptions.types as string[] | undefined) ?? [];
      for (const t of ["bun-types", "node"]) {
        if (!types.includes(t)) { types.push(t); changed = true; }
      }
      obj.compilerOptions.types = types;
      if (obj.compilerOptions.erasableSyntaxOnly === true) {
        delete obj.compilerOptions.erasableSyntaxOnly;
        changed = true;
      }

      if (!changed) return;
      await Bun.write(p, JSON.stringify(obj, null, 2));
      console.log(`  ~ ${name} (excluded "backend", added bun-types)`);
      return;
    } catch {
      // Silent fallback to next candidate
    }
  }
}

/** Detect if we're inside a monorepo workspace that includes a local bunview. */
async function resolveBunviewSpec(dir: string): Promise<string> {
  let current = dir;
  for (let i = 0; i < 8; i++) {
    const parent = path.dirname(current);
    if (parent === current) break;
    const pkgPath = path.join(parent, "package.json");
    if (await Bun.file(pkgPath).exists()) {
      try {
        const pkg = await Bun.file(pkgPath).json() as { workspaces?: string[] };
        if (pkg.workspaces?.length) {
          // Heuristic: if workspace includes packages/* and packages/bunview exists, use workspace:*
          const bunviewPkg = path.join(parent, "packages", "bunview", "package.json");
          if (await Bun.file(bunviewPkg).exists()) return "workspace:*";
        }
      } catch {}
    }
    current = parent;
  }
  return "latest";
}

async function patchPackageJson(dir: string) {
  const pkgPath = path.join(dir, "package.json");
  const pkg = await Bun.file(pkgPath).json() as Record<string, any>;

  pkg.dependencies = pkg.dependencies ?? {};
  pkg.dependencies["bunview"] = await resolveBunviewSpec(dir);
  pkg.devDependencies = pkg.devDependencies ?? {};
  pkg.devDependencies["bun-types"] = pkg.devDependencies["bun-types"] ?? "latest";

  const existingBuild = pkg.scripts?.["build"];
  const existingDev   = pkg.scripts?.["dev"];

  pkg.scripts = pkg.scripts ?? {};
  if (existingBuild) pkg.scripts["build:ui"] = existingBuild;
  if (existingDev)   pkg.scripts["dev:ui"]   = existingDev;

  pkg.scripts["dev"]   = "bunview dev";
  pkg.scripts["build"] = existingBuild ? "bun run build:ui && bunview build" : "bunview build";
  pkg.scripts["start"] = "bunview start";

  await Bun.write(pkgPath, JSON.stringify(pkg, null, 2));
  console.log(`  ~ package.json (added bunview scripts)`);
}

async function runBunInstall(dir: string): Promise<boolean> {
  console.log(`\n[bunview] Running \`bun install\`…`);
  const proc = Bun.spawn(["bun", "install"], {
    cwd: dir,
    stdout: "inherit",
    stderr: "inherit",
    env: { ...process.env, BUNVIEW_SKIP_BUILD: "1" },  // defer host download
  });
  return (await proc.exited) === 0;
}

export async function runInit(args: string[]) {
  const dir = process.cwd();
  const project = await detectProject(dir);
  const skipInstall = args.includes("--no-install");
  const force   = args.includes("--force") || args.includes("-f");
  const dryRun  = args.includes("--dry-run");

  if (dryRun) console.log("[bunview] DRY RUN — no files will be written\n");

  if (project.hasBunviewConfig && !force) {
    console.log(`[bunview] This project already has a bunview config. Nothing to do.`);
    console.log(`          Use \`bunview init --force\` to overwrite.`);
    return;
  }

  if (project.serverFramework) {
    console.error(
      `[bunview] ❌ ${project.serverFramework} detected.\n\n` +
      `  ${project.serverFramework} includes its own server — it is not compatible with bunview.\n` +
      `  Bunview replaces the server: it serves static files and provides native IPC.\n\n` +
      `  Compatible frameworks (frontend-only):\n` +
      `    Vite      + React / Vue / Svelte / Solid / Preact / Lit / Qwik\n` +
      `    Rsbuild   + React / Vue / Svelte / Solid\n` +
      `    Angular CLI (SPA mode)\n` +
      `    Parcel\n` +
      `    Create React App\n`,
    );
    process.exit(1);
  }

  if (project.buildTool !== "none") {
    const label = projectLabel(project);
    const outDir = `./${project.outDir}`;
    console.log(`[bunview] Detected: ${label}`);
    console.log(`[bunview] Build output: ${outDir}`);
    console.log(`[bunview] Adding bunview to existing project...\n`);

    await writeFile(dir, "bunview.config.ts", templateConfig(outDir), dryRun);
    await writeFile(dir, "backend/main.ts", templateBackend(project.name, outDir), dryRun);
    await writeFile(dir, "backend/commands.ts", templateCommands(), dryRun);
    await writeFile(dir, "backend/internal.ts", templateInternal(), dryRun);
    await writeFile(dir, "backend/tsconfig.json", templateBackendTsconfig(), dryRun);
    if (!dryRun) {
      await patchPackageJson(dir);
      await patchAppTsconfig(dir);
    } else {
      console.log(`  [would patch]  package.json`);
      console.log(`  [would patch]  tsconfig.app.json`);
    }

    console.log(`\n[bunview] ✅ bunview added to ${label} project`);
    console.log(`\n  Files created:`);
    console.log(`    bunview.config.ts    — points to ${outDir} (your existing build output)`);
    console.log(`    backend/main.ts      — backend entry (commands, window config)`);
    console.log(`\n  package.json updated:`);
    console.log(`    + "bunview" dependency`);
    console.log(`    + scripts: dev, build:desktop, start`);
    console.log(`\n  Your framework config was NOT modified.`);
    console.log(`  bunview reads from "${outDir}" — build your frontend first, then run bunview.`);

    if (!skipInstall && !dryRun) {
      await runBunInstall(dir);
    }

    console.log(`\n  Next steps:`);
    if (skipInstall || dryRun) console.log(`    bun install`);
    console.log(`    bun run build:ui && bun run dev`);
    return;
  }

  console.log(`[bunview] No framework detected — creating a vanilla bunview project.\n`);

  const name = project.name;

  const files: Record<string, string> = {
    "bunview.config.ts": templateConfig("./frontend"),
    "backend/main.ts": templateBackendVanilla(name),
    "backend/commands.ts": templateCommands(),
    "backend/internal.ts": templateInternal(),
    "frontend/index.html": templateHtml(name),
    "frontend/app.js": templateAppJs(),
  };

  if (!project.hasPackageJson) {
    files["package.json"] = templatePackageJson(name);
  }

  for (const [rel, content] of Object.entries(files)) {
    await writeFile(dir, rel, content);
  }

  console.log(`\n[bunview] ✅ Project "${name}" initialized`);

  if (!skipInstall) {
    await runBunInstall(dir);
    console.log(`\n          bun run dev`);
  } else {
    console.log(`          bun install`);
    console.log(`          bun run dev`);
  }
}
