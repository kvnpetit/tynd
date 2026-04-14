import path from "path";
import { BunviewConfigSchema } from "./config-schema";
import type { BunviewConfig } from "./types";

/**
 * Runtime loader for bunview.config.ts. Called by createApp() at startup.
 * Silently returns `null` if no config found — runtime then falls back to whatever
 * was passed programmatically to createApp().
 */
export async function loadRuntimeConfig(): Promise<BunviewConfig | null> {
  const cwd = process.cwd();
  for (const name of ["bunview.config.ts", "bunview.config.js"]) {
    const configPath = path.join(cwd, name);
    if (!await Bun.file(configPath).exists()) continue;

    const mod = await import(configPath);
    const raw = mod.default ?? mod;

    const parsed = BunviewConfigSchema.safeParse(raw);
    if (!parsed.success) {
      console.error(`[bunview] Invalid ${name}:`);
      for (const issue of parsed.error.issues) {
        const p = issue.path.length ? issue.path.join(".") : "(root)";
        console.error(`  • ${p}: ${issue.message}`);
      }
      process.exit(1);
    }
    return parsed.data as BunviewConfig;
  }
  return null;
}
