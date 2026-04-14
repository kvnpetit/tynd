import { intro, outro, spinner, confirm, isCancel, cancel } from "@clack/prompts";
import { fileURLToPath } from "url";
import path from "path";
import { log, c } from "../utils/colors";

async function fetchLatestVersion(): Promise<string | null> {
  try {
    const res = await fetch("https://registry.npmjs.org/bunview/latest");
    if (!res.ok) return null;
    const data = await res.json() as { version: string };
    return data.version ?? null;
  } catch { return null; }
}

async function readCurrentVersion(): Promise<string> {
  const pkgPath = path.join(fileURLToPath(new URL("../../", import.meta.url)), "package.json");
  try {
    const pkg = await Bun.file(pkgPath).json() as { version: string };
    return pkg.version;
  } catch { return "0.0.0"; }
}

/** Compare semver-like x.y.z strings. Returns 1, 0, -1. */
function compareSemver(a: string, b: string): number {
  const pa = a.split(".").map((n) => parseInt(n, 10));
  const pb = b.split(".").map((n) => parseInt(n, 10));
  for (let i = 0; i < 3; i++) {
    const da = pa[i] ?? 0;
    const db = pb[i] ?? 0;
    if (da > db) return 1;
    if (da < db) return -1;
  }
  return 0;
}

export async function runUpgrade(args: string[]) {
  intro(c.bold("bunview upgrade"));

  const s = spinner();
  s.start("Checking latest version…");
  const current = await readCurrentVersion();
  const latest = await fetchLatestVersion();
  s.stop("Version check complete");

  if (!latest) {
    log.error("Could not reach npm registry.");
    process.exit(1);
  }

  console.log(`  current: ${c.dim(current)}`);
  console.log(`  latest:  ${c.cyan(latest)}`);

  const cmp = compareSemver(latest, current);
  if (cmp <= 0) {
    outro(c.green("Already on latest version."));
    return;
  }

  if (!args.includes("--yes") && !args.includes("-y")) {
    const ok = await confirm({
      message: `Upgrade bunview ${current} → ${latest}?`,
      initialValue: true,
    });
    if (isCancel(ok) || !ok) { cancel("Cancelled."); return; }
  }

  const installSp = spinner();
  installSp.start(`Installing bunview@${latest}…`);
  const proc = Bun.spawn(["bun", "install", `bunview@${latest}`], {
    cwd: process.cwd(),
    stdout: "ignore",
    stderr: "inherit",
  });
  const code = await proc.exited;
  if (code !== 0) {
    installSp.stop(c.red("Install failed"));
    process.exit(1);
  }
  installSp.stop("Installed");

  outro(`${c.green("✓")} Upgraded to bunview ${latest}`);
}
