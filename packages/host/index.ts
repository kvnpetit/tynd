/**
 * Typed helper for resolving the right native host binary at runtime.
 *
 * `@tynd/host` is a wrapper package: the actual `tynd-full` / `tynd-lite`
 * binaries live in the platform-specific sub-packages
 * (`@tynd/host-darwin-x64`, `@tynd/host-linux-arm64`, etc.) installed
 * automatically by npm/bun via `optionalDependencies`.
 *
 * The CLI's `findBinary` does its own walk-the-node_modules-tree lookup
 * for compatibility with workspace + hoisted layouts. Third-party tooling
 * that just wants the binary path can use the helper below.
 */

import { createRequire } from "node:module";

const require = createRequire(import.meta.url);

export type Runtime = "full" | "lite";

/**
 * Sub-package name for the current host (`@tynd/host-<plat>-<arch>`).
 *
 * **Single source of truth** for the platform-package naming scheme.
 * The CLI's `cli/src/lib/detect/binary.ts` mirrors this format — keep
 * the two in sync if the scheme ever changes.
 */
export function hostSubPackageName(): string {
  const platform = process.platform;
  const arch = process.arch === "arm64" ? "arm64" : "x64";
  return `@tynd/host-${platform}-${arch}`;
}

/**
 * Resolve the absolute path of a Tynd host binary for the current
 * platform. Throws if the matching `@tynd/host-<plat>-<arch>` sub-package
 * isn't installed (which means npm skipped it because of `os` / `cpu`
 * mismatch — the platform isn't supported).
 */
export function resolveHostBinary(runtime: Runtime): string {
  const ext = process.platform === "win32" ? ".exe" : "";
  return require.resolve(`${hostSubPackageName()}/bin/tynd-${runtime}${ext}`);
}
