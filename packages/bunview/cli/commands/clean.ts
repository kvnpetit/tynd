import path from "path";
import { existsSync, rmSync } from "fs";
import { confirm, isCancel, cancel } from "@clack/prompts";
import { log, c } from "../utils/colors";
import { loadConfig } from "../utils/config";

export async function runClean(args: string[]) {
  const cwd = process.cwd();
  const yes = args.includes("--yes") || args.includes("-y");

  let outDir = "release";
  try { outDir = (await loadConfig()).outDir ?? "release"; } catch { /* no config */ }

  const targets = [
    path.join(cwd, outDir),
    path.join(cwd, "dist"),
    path.join(cwd, ".bunview-tmp"),
  ].filter(existsSync);

  if (targets.length === 0) {
    log.info("Nothing to clean — no build artifacts found.");
    return;
  }

  log.info("Will delete:");
  for (const t of targets) console.log(`  ${c.gray(path.relative(cwd, t))}`);

  if (!yes) {
    const confirmed = await confirm({
      message: "Proceed with deletion?",
      initialValue: false,
    });
    if (isCancel(confirmed) || !confirmed) {
      cancel("Cancelled.");
      return;
    }
  }

  for (const t of targets) {
    try {
      rmSync(t, { recursive: true, force: true });
      log.ok(`Removed ${path.relative(cwd, t)}`);
    } catch (err) {
      log.error(`Failed to remove ${path.relative(cwd, t)}: ${err}`);
    }
  }
}
