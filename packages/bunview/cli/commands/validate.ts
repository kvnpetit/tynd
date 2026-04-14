import path from "path";
import { existsSync } from "fs";
import { log, c } from "../utils/colors";

export async function runValidate(args: string[]) {
  const json = args.includes("--json");
  const issues: { level: "error" | "warn" | "info"; message: string; path?: string }[] = [];

  // 1. Config file present + parses
  try {
    const { loadConfig } = await import("../utils/config");
    const cfg = await loadConfig();
    issues.push({ level: "info", message: "bunview.config.ts parses and validates" });

    // 2. Entry file exists
    const entryAbs = path.resolve(process.cwd(), cfg.entry);
    if (!existsSync(entryAbs)) {
      issues.push({ level: "error", message: `entry file not found`, path: cfg.entry });
    } else {
      issues.push({ level: "info", message: `entry file exists`, path: cfg.entry });
    }

    // 3. Frontend directory (if specified)
    if (cfg.frontend) {
      const frontAbs = path.resolve(process.cwd(), cfg.frontend);
      if (!existsSync(frontAbs)) {
        issues.push({ level: "warn", message: "frontend directory missing — run your UI build first", path: cfg.frontend });
      } else {
        issues.push({ level: "info", message: "frontend directory exists", path: cfg.frontend });
      }
    }

    // 4. Icon (if specified)
    if (cfg.icon) {
      const iconAbs = path.resolve(process.cwd(), cfg.icon);
      if (!existsSync(iconAbs)) issues.push({ level: "warn", message: "icon file missing", path: cfg.icon });
    }

    // 5. Code signing cert (Windows)
    if (cfg.codeSigning?.windows?.certificate) {
      const cert = path.resolve(process.cwd(), cfg.codeSigning.windows.certificate);
      if (!existsSync(cert)) issues.push({ level: "error", message: "Windows signing certificate not found", path: cfg.codeSigning.windows.certificate });
    }

    // 6. Env var placeholders resolved
    const checkEnv = (value: string | undefined, label: string) => {
      if (!value) return;
      const match = value.match(/\$\{(\w+)\}/);
      if (match && !process.env[match[1]!]) {
        issues.push({ level: "warn", message: `${label} references $\{${match[1]}\} which is unset` });
      }
    };
    checkEnv(cfg.codeSigning?.windows?.password, "Windows cert password");
    checkEnv(cfg.codeSigning?.macos?.notarize?.appleId,  "Apple ID");
    checkEnv(cfg.codeSigning?.macos?.notarize?.teamId,   "Apple Team ID");
    checkEnv(cfg.codeSigning?.macos?.notarize?.password, "Apple app password");
  } catch {
    issues.push({ level: "error", message: "bunview.config.ts invalid or missing" });
  }

  if (json) {
    console.log(JSON.stringify({ issues, ok: !issues.some((i) => i.level === "error") }, null, 2));
    return;
  }

  console.log(`\n${c.bold("bunview validate")}\n`);
  for (const i of issues) {
    const icon = i.level === "error" ? c.red("✗") : i.level === "warn" ? c.yellow("⚠") : c.green("✓");
    const pathStr = i.path ? c.dim(` (${i.path})`) : "";
    console.log(`  ${icon} ${i.message}${pathStr}`);
  }
  const errors = issues.filter((i) => i.level === "error").length;
  const warns  = issues.filter((i) => i.level === "warn").length;
  console.log("");
  if (errors > 0) {
    log.error(`${errors} error(s), ${warns} warning(s)`);
    process.exit(1);
  } else if (warns > 0) {
    log.warn(`${warns} warning(s)`);
  } else {
    log.ok("Config is valid and all referenced files exist.");
  }
}
