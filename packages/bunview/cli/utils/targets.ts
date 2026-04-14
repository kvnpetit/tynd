import type { Target } from "./types";

const ALL_TARGETS: Target[] = [
  "windows-x64", "windows-arm64",
  "linux-x64",   "linux-arm64",
  "macos-x64",   "macos-arm64",
];

export function currentTarget(): Target {
  const arch = process.arch === "arm64" ? "arm64" : "x64";
  if (process.platform === "win32")  return `windows-${arch}`;
  if (process.platform === "darwin") return `macos-${arch}`;
  return `linux-${arch}`;
}

/**
 * Parse CLI args for a target flag. Checks both the passed args array AND
 * `process.argv` directly (cac rewrites `--foo-bar` to camelCase `fooBar`,
 * which breaks kebab-case lookup in the parsed options object).
 * Falls back to the current platform.
 */
export function resolveTarget(args: string[]): Target {
  const all = [...args, ...process.argv.slice(2)];

  for (const t of ALL_TARGETS) {
    if (all.some((a) => a === `--${t}` || a === `--${t}=true` || a === t)) return t;
  }

  const archIsArm = process.arch === "arm64";
  if (all.includes("--windows")) return archIsArm ? "windows-arm64" : "windows-x64";
  if (all.includes("--linux"))   return archIsArm ? "linux-arm64"   : "linux-x64";
  if (all.includes("--macos"))   return archIsArm ? "macos-arm64"   : "macos-x64";

  return currentTarget();
}

export function bunTarget(t: Target): string {
  switch (t) {
    case "windows-x64":   return "bun-windows-x64";
    case "windows-arm64": return "bun-windows-arm64";
    case "linux-x64":     return "bun-linux-x64";
    case "linux-arm64":   return "bun-linux-aarch64";
    case "macos-x64":     return "bun-darwin-x64";
    case "macos-arm64":   return "bun-darwin-arm64";
  }
}
