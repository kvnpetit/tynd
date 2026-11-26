// Mirrors workspace crates into npm packages as standalone sources: copies a
// crate and rewrites its manifest so it builds without the monorepo workspace.
// Shared by the `@tynd/host-src` and `@tynd/plugin-sdk` sync scripts.
import { cpSync } from "node:fs";
import { basename, join } from "node:path";
import { parse, stringify } from "smol-toml";

/** Repository root. */
export const ROOT = join(import.meta.dir, "..", "..", "..");

const VENDOR_EXCLUDE = new Set([
  "target",
  "node_modules",
  ".git",
  ".cargo",
  ".idea",
  ".vscode",
  "tests",
  "benches",
]);

export function copyVendored(src: string, dest: string): void {
  cpSync(src, dest, {
    recursive: true,
    filter: (s: string) => {
      const name = basename(s);
      if (VENDOR_EXCLUDE.has(name)) return false;
      if (name.endsWith(".profraw")) return false;
      return true;
    },
  });
}

export type TomlTable = Record<string, unknown>;

export interface WorkspaceValues {
  pkg: TomlTable;
  deps: TomlTable;
  lints: TomlTable;
}

export async function readWorkspaceValues(): Promise<WorkspaceValues> {
  const text = await Bun.file(join(ROOT, "Cargo.toml")).text();
  const doc = parse(text) as TomlTable;
  const ws = (doc.workspace ?? {}) as TomlTable;
  return {
    pkg: (ws.package ?? {}) as TomlTable,
    deps: (ws.dependencies ?? {}) as TomlTable,
    lints: (ws.lints ?? {}) as TomlTable,
  };
}

export function isTable(value: unknown): value is TomlTable {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function hasWorkspaceFlag(v: unknown): v is TomlTable {
  return isTable(v) && v.workspace === true;
}

function resolveDepRef(value: TomlTable, wsDeps: TomlTable, key: string): unknown {
  const { workspace: _w, ...localExtras } = value;
  const wsEntry = wsDeps[key];
  if (wsEntry === undefined) return localExtras;
  if (typeof wsEntry === "string") {
    return Object.keys(localExtras).length === 0 ? wsEntry : { version: wsEntry, ...localExtras };
  }
  if (typeof wsEntry === "object" && wsEntry !== null) {
    return { ...(wsEntry as TomlTable), ...localExtras };
  }
  return wsEntry;
}

function flattenPackage(pkg: TomlTable, wsPkg: TomlTable): TomlTable {
  const out: TomlTable = {};
  for (const [k, v] of Object.entries(pkg)) {
    if (k === "readme") continue;
    if (hasWorkspaceFlag(v)) {
      const wsVal = wsPkg[k];
      if (wsVal !== undefined) out[k] = wsVal;
      continue;
    }
    out[k] = v;
  }
  return out;
}

export function flattenDeps(deps: TomlTable, wsDeps: TomlTable): TomlTable {
  const out: TomlTable = {};
  for (const [k, v] of Object.entries(deps)) {
    out[k] = hasWorkspaceFlag(v) ? resolveDepRef(v, wsDeps, k) : v;
  }
  return out;
}

/** Point a dependency at `path`, keeping its other keys (features, optional...). */
export function pinPath(value: unknown, path: string): TomlTable {
  const extras = isTable(value) ? value : {};
  const { path: _path, workspace: _workspace, ...rest } = extras;
  return { path, ...rest };
}

// `lints` is kept: a mirror that carries the root [workspace.lints] table
// resolves `[lints] workspace = true` to the same lint policy. A mirror with no
// workspace above it asks for `inlineLints` instead.
const WORKSPACE_ONLY_SECTIONS = ["bench", "test", "example", "dev-dependencies"];

export function omitTomlKeys(table: TomlTable, keys: readonly string[]): TomlTable {
  const omitted = new Set(keys);
  return Object.fromEntries(Object.entries(table).filter(([key]) => !omitted.has(key)));
}

export type DepRewrite = (deps: TomlTable) => TomlTable;

export function mapDepSections(table: TomlTable, rewrite: DepRewrite): void {
  for (const key of ["dependencies", "build-dependencies"] as const) {
    const section = table[key];
    if (isTable(section)) table[key] = rewrite(section);
  }
}

export function mapTargetDepSections(doc: TomlTable, rewrite: DepRewrite): void {
  const targets = doc.target;
  if (!isTable(targets)) return;
  for (const [cfg, cfgVal] of Object.entries(targets)) {
    if (!isTable(cfgVal)) continue;
    const cfgTable = omitTomlKeys(cfgVal, ["dev-dependencies"]);
    mapDepSections(cfgTable, rewrite);
    targets[cfg] = cfgTable;
  }
}

export interface MirrorOptions {
  /**
   * Replace `[lints] workspace = true` with the root lint table, for a crate
   * mirrored where no Cargo workspace sits above it.
   */
  inlineLints?: boolean;
  /** Dependency name → path of its own mirror, relative to the mirrored crate. */
  siblingPaths?: Readonly<Record<string, string>>;
}

/** Parse a crate manifest, drop workspace-only sections and inline `[package]` workspace values. */
export function parseStandaloneManifest(
  src: string,
  ws: WorkspaceValues,
  options: MirrorOptions = {},
): TomlTable {
  const doc = omitTomlKeys(parse(src) as TomlTable, WORKSPACE_ONLY_SECTIONS);
  if (isTable(doc.package)) {
    doc.package = flattenPackage(doc.package, ws.pkg);
  }
  if (options.inlineLints && hasWorkspaceFlag(doc.lints)) {
    doc.lints = ws.lints;
  }
  return doc;
}

/** Standalone manifest of a mirrored crate. */
function flattenVendoredCargoToml(
  src: string,
  ws: WorkspaceValues,
  options: MirrorOptions,
): string {
  const doc = parseStandaloneManifest(src, ws, options);
  const siblingPaths = options.siblingPaths ?? {};
  const pinSiblings = (table: TomlTable): TomlTable => {
    const out: TomlTable = {};
    for (const [k, v] of Object.entries(table)) {
      const sibling = siblingPaths[k];
      out[k] = sibling === undefined ? v : pinPath(v, sibling);
    }
    return out;
  };
  const flattenSection: DepRewrite = (section) => pinSiblings(flattenDeps(section, ws.deps));
  mapDepSections(doc, flattenSection);
  mapTargetDepSections(doc, flattenSection);
  return stringify(doc);
}

/** Copy a crate into the mirror and make its manifest standalone. */
export async function vendorCrate(
  src: string,
  dest: string,
  ws: WorkspaceValues,
  options: MirrorOptions = {},
): Promise<void> {
  copyVendored(src, dest);
  const tomlPath = join(dest, "Cargo.toml");
  const raw = await Bun.file(tomlPath).text();
  await Bun.write(tomlPath, flattenVendoredCargoToml(raw, ws, options));
}
