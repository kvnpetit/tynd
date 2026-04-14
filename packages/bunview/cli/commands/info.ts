import path from "path";
import { existsSync } from "fs";
import { fileURLToPath } from "url";
import { c, log } from "../utils/colors";
import { detectProject, projectLabel } from "../utils/detect";
import { currentTarget } from "../utils/targets";

async function readBunviewVersion(): Promise<string> {
  const pkgPath = path.join(fileURLToPath(new URL("../../", import.meta.url)), "package.json");
  try {
    const pkg = await Bun.file(pkgPath).json() as { version: string };
    return pkg.version;
  } catch { return "unknown"; }
}

function fmt(label: string, value: string): string {
  return `  ${c.dim(label.padEnd(20))} ${value}`;
}

export async function runInfo(args: string[]) {
  const json = args.includes("--json");
  const cwd = process.cwd();
  const bunviewVersion = await readBunviewVersion();
  const project = await detectProject(cwd);
  const target = currentTarget();

  // Try to read bunview.config.ts without exiting on failure
  let configInfo: { entry?: string; frontend?: string; outDir?: string; urlScheme?: string; hasSigning?: boolean } = {};
  try {
    const { loadConfig } = await import("../utils/config");
    const cfg = await loadConfig();
    configInfo = {
      entry:      cfg.entry,
      frontend:   cfg.frontend,
      outDir:     cfg.outDir,
      urlScheme:  cfg.urlScheme?.name,
      hasSigning: !!cfg.codeSigning,
    };
  } catch { /* no config yet */ }

  // Detect pre-built webview-host
  const bunviewRoot = fileURLToPath(new URL("../../", import.meta.url));
  const ext = target.startsWith("windows") ? ".exe" : "";
  const hostBin = path.join(bunviewRoot, "bin", target, `webview-host${ext}`);
  const hostExists = existsSync(hostBin);

  const info = {
    bunview: bunviewVersion,
    bun:     Bun.version,
    platform: `${process.platform}-${process.arch}`,
    target,
    project: {
      name:      project.name,
      framework: projectLabel(project),
      outDir:    project.outDir,
      hasConfig: project.hasBunviewConfig,
    },
    config: configInfo,
    hostBinary: { path: hostBin, exists: hostExists },
  };

  if (json) {
    console.log(JSON.stringify(info, null, 2));
    return;
  }

  console.log(`\n${c.bold("bunview info")}`);
  console.log(fmt("bunview version",  info.bunview));
  console.log(fmt("bun runtime",      info.bun));
  console.log(fmt("platform",         info.platform));
  console.log(fmt("current target",   info.target));
  console.log("");
  console.log(`  ${c.bold("Project")}`);
  console.log(fmt("name",       info.project.name));
  console.log(fmt("framework",  info.project.framework));
  console.log(fmt("out dir",    info.project.outDir));
  console.log(fmt("has config", info.project.hasConfig ? c.green("yes") : c.yellow("no — run `bunview init`")));
  if (configInfo.entry) {
    console.log("");
    console.log(`  ${c.bold("Config")}`);
    console.log(fmt("entry",     configInfo.entry));
    if (configInfo.frontend)  console.log(fmt("frontend",  configInfo.frontend));
    if (configInfo.outDir)    console.log(fmt("outDir",    configInfo.outDir));
    if (configInfo.urlScheme) console.log(fmt("urlScheme", configInfo.urlScheme));
    console.log(fmt("code signing", configInfo.hasSigning ? c.green("configured") : c.dim("not set")));
  }
  console.log("");
  console.log(`  ${c.bold("Runtime")}`);
  console.log(fmt("webview-host", hostExists ? c.green(path.relative(cwd, hostBin)) : c.red("not downloaded")));
  if (!hostExists) log.warn(`Host binary missing — run \`bun install\` or \`bunview build\` to fetch it.`);
  console.log("");
}
