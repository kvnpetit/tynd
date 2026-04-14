import { log, c } from "../utils/colors";
import { spinner, intro, outro } from "@clack/prompts";
import { runBuild } from "./build";
import type { Target } from "../utils/types";

const TARGETS: Target[] = [
  "windows-x64", "windows-arm64",
  "linux-x64",   "linux-arm64",
  "macos-x64",   "macos-arm64",
];

/** Build for all 6 desktop targets sequentially. */
export async function runBuildAll(_args: string[]) {
  intro(c.bold("bunview build --all") + c.dim(" (6 targets)"));

  const results: { target: Target; ok: boolean; error?: string }[] = [];

  for (const target of TARGETS) {
    const s = spinner();
    s.start(`Building ${target}…`);
    try {
      // Pass the target as a --<target> flag to runBuild
      await runBuild([`--${target}`]);
      results.push({ target, ok: true });
      s.stop(`${c.green("✓")} ${target}`);
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      results.push({ target, ok: false, error: msg });
      s.stop(`${c.red("✗")} ${target}`);
    }
  }

  const failed = results.filter((r) => !r.ok);
  if (failed.length === 0) {
    outro(c.green(`All ${TARGETS.length} targets built successfully.`));
  } else {
    log.error(`${failed.length}/${TARGETS.length} targets failed:`);
    for (const f of failed) console.error(`  ${c.red("•")} ${f.target}: ${f.error}`);
    process.exit(1);
  }
}
