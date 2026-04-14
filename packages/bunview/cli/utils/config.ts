import path from "path";
import { BunviewConfigSchema } from "../../src/config-schema";
import { log, c } from "./colors";
import type { BunviewConfig } from "../../src/types";

function argvConfigPath(): string | null {
  const argv = process.argv;
  for (let i = 2; i < argv.length; i++) {
    const a = argv[i]!;
    if (a === "--config" || a === "-c") return argv[i + 1] ?? null;
    if (a.startsWith("--config=")) return a.slice("--config=".length);
  }
  return null;
}

export async function loadConfig(): Promise<BunviewConfig> {
  const cwd = process.cwd();
  const explicit = argvConfigPath();

  const candidates = explicit
    ? [path.resolve(cwd, explicit)]
    : [path.join(cwd, "bunview.config.ts"), path.join(cwd, "bunview.config.js")];

  for (const configPath of candidates) {
    if (!await Bun.file(configPath).exists()) continue;

    const mod = await import(configPath);
    const raw = mod.default ?? mod;

    const parsed = BunviewConfigSchema.safeParse(raw);
    if (!parsed.success) {
      log.error(`Invalid ${path.basename(configPath)}:`);
      for (const issue of parsed.error.issues) {
        const pathStr = issue.path.length ? issue.path.join(".") : "(root)";
        console.error(`  ${c.red("•")} ${c.bold(pathStr)}: ${issue.message}`);
      }
      process.exit(1);
    }
    return parsed.data as BunviewConfig;
  }

  if (explicit) {
    log.error(`Config file not found: ${explicit}`);
    process.exit(1);
  }
  log.error(`No bunview.config.ts found in ${cwd}`);
  console.error(
    `\n  Create a bunview.config.ts:\n\n` +
    `    import { defineConfig } from "bunview";\n` +
    `    export default defineConfig({ entry: "backend/main.ts" });\n`,
  );
  process.exit(1);
}

export async function readPackageName(cwd: string): Promise<string | null> {
  const pkgPath = path.join(cwd, "package.json");
  if (await Bun.file(pkgPath).exists()) {
    const pkg = await Bun.file(pkgPath).json() as { name?: string };
    return pkg.name ?? null;
  }
  return null;
}
