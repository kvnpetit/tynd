#!/usr/bin/env bun
import { existsSync } from "node:fs";
/**
 * Usage: bun run scripts/new-version.ts v0.2
 *
 * Copies the current latest docs directory to a new versioned folder.
 * You still have to:
 *   1. Add the new version to `src/lib/versions.ts` (and flip `latest`).
 *   2. Review / rewrite the duplicated content for breaking changes.
 */
import { cp, readdir } from "node:fs/promises";
import { join } from "node:path";

const [, , rawVersion] = process.argv;
if (!rawVersion) {
  console.error("Usage: bun run scripts/new-version.ts <version>  (e.g. v0.2)");
  process.exit(1);
}
const newVersion = rawVersion.startsWith("v") ? rawVersion : `v${rawVersion}`;

const contentDir = join(import.meta.dir, "..", "src", "content");
const versions = (await readdir(contentDir, { withFileTypes: true }))
  .filter((d) => d.isDirectory() && /^v\d+/.test(d.name))
  .map((d) => d.name)
  .sort();

if (versions.length === 0) {
  console.error("No existing version folder under src/content/");
  process.exit(1);
}

const source = versions[versions.length - 1];
const target = join(contentDir, newVersion);
if (existsSync(target)) {
  console.error(`${newVersion} already exists — aborting`);
  process.exit(1);
}

await cp(join(contentDir, source), target, { recursive: true });

console.log(`Copied src/content/${source} -> src/content/${newVersion}`);
console.log("Next steps:");
console.log(
  "  1. Edit src/lib/versions.ts to add the new version + flip 'latest'",
);
console.log(
  `  2. Update public/_redirects — swap /docs/v0.1 for /docs/${newVersion} on the 4 rules`,
);
console.log("  3. Review the copied MDX files for breaking changes");
