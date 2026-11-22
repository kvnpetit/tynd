import { existsSync, readdirSync } from "node:fs";
import path from "node:path";

export type ReleaseScope = "framework" | "plugins";

export interface VersionedPackage {
  name: string;
  path: string;
  version: string;
  /** Exact pins on `@tynd/tooling*` packages, which must follow the release version. */
  toolingPins?: Record<string, string>;
}

const TAG_PREFIX: Record<ReleaseScope, string> = {
  framework: "v",
  plugins: "plugins-v",
};

export const FRAMEWORK_PACKAGES = [
  "packages/cli",
  "packages/core",
  "packages/host-src",
  "packages/mcp",
  "packages/plugin-sdk",
  "packages/test",
  "npm/tooling",
  "npm/tooling-darwin-arm64",
  "npm/tooling-darwin-x64",
  "npm/tooling-linux-arm64",
  "npm/tooling-linux-x64",
  "npm/tooling-win32-arm64",
  "npm/tooling-win32-x64",
] as const;

const VERSION_RE = /^\d+\.\d+\.\d+(?:-[0-9A-Za-z]+(?:[.-][0-9A-Za-z]+)*)?$/;

export function versionFromTag(scope: ReleaseScope, tag: string): string {
  const prefix = TAG_PREFIX[scope];
  if (!tag.startsWith(prefix)) {
    throw new Error(`Expected ${scope} tag "${prefix}<version>", got "${tag}"`);
  }
  const version = tag.slice(prefix.length);
  if (!VERSION_RE.test(version)) {
    throw new Error(`Invalid release version "${version}" in tag "${tag}"`);
  }
  return version;
}

export function validateReleaseVersions(
  scope: ReleaseScope,
  tag: string,
  packages: readonly VersionedPackage[],
  cargoWorkspaceVersion?: string,
): string {
  const expected = versionFromTag(scope, tag);
  const mismatches = packages
    .filter((pkg) => pkg.version !== expected)
    .map((pkg) => `${pkg.path} (${pkg.name}): ${pkg.version}`);
  for (const pkg of packages) {
    for (const [dependency, spec] of Object.entries(pkg.toolingPins ?? {})) {
      if (spec !== expected) mismatches.push(`${pkg.path} (${pkg.name}) -> ${dependency}: ${spec}`);
    }
  }

  if (cargoWorkspaceVersion !== undefined && cargoWorkspaceVersion !== expected) {
    mismatches.push(`Cargo.toml ([workspace.package]): ${cargoWorkspaceVersion}`);
  }
  if (mismatches.length > 0) {
    throw new Error(
      [`Release tag ${tag} expects version ${expected}.`, "Mismatches:", ...mismatches].join("\n"),
    );
  }
  return expected;
}

async function readPackage(repoRoot: string, relativeDir: string): Promise<VersionedPackage> {
  const manifestPath = path.join(repoRoot, relativeDir, "package.json");
  const manifest = (await Bun.file(manifestPath).json()) as {
    name?: unknown;
    version?: unknown;
    dependencies?: Record<string, string>;
    optionalDependencies?: Record<string, string>;
  };
  if (typeof manifest.name !== "string" || typeof manifest.version !== "string") {
    throw new Error(
      `${path.relative(repoRoot, manifestPath)} needs string name and version fields`,
    );
  }
  const toolingPins = Object.fromEntries(
    Object.entries({ ...manifest.dependencies, ...manifest.optionalDependencies }).filter(
      ([dependency]) => dependency === "@tynd/tooling" || dependency.startsWith("@tynd/tooling-"),
    ),
  );
  return {
    name: manifest.name,
    path: relativeDir.replaceAll("\\", "/"),
    version: manifest.version,
    toolingPins,
  };
}

function cargoWorkspaceVersion(source: string): string {
  const workspacePackage = source.match(
    /\[workspace\.package\]([\s\S]*?)(?=\n\[[^\]]+\]|\s*$)/,
  )?.[1];
  const version = workspacePackage?.match(/^\s*version\s*=\s*"([^"]+)"/m)?.[1];
  if (!version) throw new Error("Cargo.toml is missing [workspace.package].version");
  return version;
}

/** Every publishable package under plugins/host (the `@tynd/plugin-<id>` API plugins). */
function hostPluginDirs(repoRoot: string): string[] {
  const root = path.join(repoRoot, "plugins", "host");
  return readdirSync(root, { withFileTypes: true })
    .filter((entry) => entry.isDirectory())
    .map((entry) => `plugins/host/${entry.name}`)
    .filter((dir) => existsSync(path.join(repoRoot, dir, "package.json")))
    .sort();
}

async function packagesForScope(
  scope: ReleaseScope,
  repoRoot: string,
): Promise<VersionedPackage[]> {
  if (scope === "framework") {
    return Promise.all(
      [...FRAMEWORK_PACKAGES, ...hostPluginDirs(repoRoot)].map((dir) => readPackage(repoRoot, dir)),
    );
  }
  const root = path.join(repoRoot, "plugins", "standalone");
  const dirs = readdirSync(root, { withFileTypes: true })
    .filter((entry) => entry.isDirectory())
    .map((entry) => `plugins/standalone/${entry.name}`)
    .filter((dir) => existsSync(path.join(repoRoot, dir, "package.json")))
    .sort();
  const packages = await Promise.all(dirs.map((dir) => readPackage(repoRoot, dir)));
  return packages;
}

export async function checkReleaseVersion(
  scope: ReleaseScope,
  tag: string,
  repoRoot = path.resolve(import.meta.dir, ".."),
): Promise<string> {
  const packages = await packagesForScope(scope, repoRoot);
  const cargoVersion =
    scope === "framework"
      ? cargoWorkspaceVersion(await Bun.file(path.join(repoRoot, "Cargo.toml")).text())
      : undefined;
  return validateReleaseVersions(scope, tag, packages, cargoVersion);
}

function parseScope(value: string | undefined): ReleaseScope {
  if (value === "framework" || value === "plugins") return value;
  throw new Error("Usage: bun scripts/check-release-version.ts <framework|plugins> <tag>");
}

if (import.meta.main) {
  const scope = parseScope(process.argv[2]);
  const tag = process.argv[3];
  if (!tag) throw new Error("Release tag is required");
  const version = await checkReleaseVersion(scope, tag);
  process.stdout.write(`release preflight: ${scope} ${version}\n`);
}
