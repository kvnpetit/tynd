// Mirrors the Rust SDK crates (`crates/plugin-sdk`, `crates/plugin-macros`)
// into this package as `native/` and `native-macros/`, the sources
// `@tynd/plugin-sdk` ships to npm. Plugin crates outside the monorepo depend on
// them by path, with no Cargo workspace above, so the manifests are made
// standalone: workspace values and the lint table are inlined.
//
// Usage: bun run scripts/sync.ts
import { existsSync, readdirSync, rmSync } from "node:fs";
import { join } from "node:path";
import { ROOT, readWorkspaceValues, vendorCrate } from "../../host-src/scripts/mirror";

const PACKAGE = join(import.meta.dir, "..");

interface Mirror {
  src: string;
  dest: string;
  /** Dependency name → path of its mirror, relative to `dest`. */
  siblingPaths?: Record<string, string>;
}

const MIRRORS: Mirror[] = [
  {
    src: join(ROOT, "crates", "plugin-sdk"),
    dest: join(PACKAGE, "native"),
    siblingPaths: { "tynd-plugin-macros": "../native-macros" },
  },
  { src: join(ROOT, "crates", "plugin-macros"), dest: join(PACKAGE, "native-macros") },
];

/** Clear a generated mirror but keep `target/`, so builds of the mirror stay incremental. */
function clearMirror(dir: string): void {
  if (!existsSync(dir)) return;
  for (const entry of readdirSync(dir)) {
    if (entry !== "target") rmSync(join(dir, entry), { recursive: true, force: true });
  }
}

for (const mirror of MIRRORS) {
  if (!existsSync(join(mirror.src, "Cargo.toml"))) {
    throw new Error(`SDK crate not found at ${mirror.src}`);
  }
}

const ws = await readWorkspaceValues();
for (const mirror of MIRRORS) {
  clearMirror(mirror.dest);
  await vendorCrate(mirror.src, mirror.dest, ws, {
    inlineLints: true,
    ...(mirror.siblingPaths ? { siblingPaths: mirror.siblingPaths } : {}),
  });
}
