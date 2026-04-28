#!/usr/bin/env bun
/**
 * Copies `packages/host-rs/` into `packages/host-src/native/`, flattening
 * every `workspace = true` reference to an explicit version/value read
 * from the root `Cargo.toml`. The output is a standalone crate that
 * builds inside the `.tynd/build/` workspace the CLI generates for
 * custom runtimes.
 *
 * Uses `smol-toml` for parsing — the produced `native/Cargo.toml` is a
 * generated artifact, so cosmetic re-serialization (lost comments / key
 * order normalised) is acceptable in exchange for correctness over the
 * full TOML grammar.
 */

import { cpSync, existsSync, mkdirSync, rmSync } from "node:fs";
import { join, relative } from "node:path";
import { parse, stringify } from "smol-toml";

const ROOT = join(import.meta.dir, "..", "..", "..");
const HOST_RS = join(ROOT, "packages", "host-rs");
const HOST_SRC_NATIVE = join(import.meta.dir, "..", "native");
const WORKSPACE_TOML = join(ROOT, "Cargo.toml");
const HOST_RS_TOML = join(HOST_RS, "Cargo.toml");

type TomlTable = Record<string, unknown>;

interface WorkspaceValues {
  pkg: TomlTable;
  deps: TomlTable;
}

async function readWorkspaceValues(): Promise<WorkspaceValues> {
  const text = await Bun.file(WORKSPACE_TOML).text();
  const doc = parse(text) as TomlTable;
  const ws = (doc.workspace ?? {}) as TomlTable;
  return {
    pkg: (ws.package ?? {}) as TomlTable,
    deps: (ws.dependencies ?? {}) as TomlTable,
  };
}

function hasWorkspaceFlag(v: unknown): v is TomlTable {
  return (
    typeof v === "object" && v !== null && !Array.isArray(v) && (v as TomlTable).workspace === true
  );
}

/** Resolve `dep.workspace = true` (possibly with extras) to a concrete inline table or version string. */
function resolveDepRef(value: TomlTable, wsDeps: TomlTable, key: string): unknown {
  const wsEntry = wsDeps[key];
  if (wsEntry === undefined) {
    // Workspace doesn't declare this dep — leave the original record minus the
    // `workspace = true` flag so cargo surfaces the error rather than us.
    const { workspace: _w, ...rest } = value;
    return rest;
  }
  // Drop the `workspace = true` flag and any keys that should always come from
  // the workspace declaration; keep per-crate overrides like `optional`.
  const { workspace: _w, ...localExtras } = value;
  if (typeof wsEntry === "string") {
    return Object.keys(localExtras).length === 0 ? wsEntry : { version: wsEntry, ...localExtras };
  }
  if (typeof wsEntry === "object" && wsEntry !== null) {
    // Workspace inline table wins for shared fields (version, features,
    // default-features); per-crate localExtras layer on top (e.g. `optional`).
    return { ...(wsEntry as TomlTable), ...localExtras };
  }
  return wsEntry;
}

function flattenPackage(pkg: TomlTable, wsPkg: TomlTable): TomlTable {
  const out: TomlTable = {};
  for (const [k, v] of Object.entries(pkg)) {
    // Drop readme — `readme = "../../README.md"` escapes the crate dir
    // once vendored into `.tynd/build/host/`. Same for `readme.workspace = true`.
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

function flattenDeps(deps: TomlTable, wsDeps: TomlTable): TomlTable {
  const out: TomlTable = {};
  for (const [k, v] of Object.entries(deps)) {
    if (hasWorkspaceFlag(v)) {
      out[k] = resolveDepRef(v, wsDeps, k);
      continue;
    }
    out[k] = v;
  }
  return out;
}

function flattenCargoToml(src: string, ws: WorkspaceValues): string {
  const doc = parse(src) as TomlTable;

  // [lints] inherits from workspace — drop it for the standalone crate.
  // biome-ignore lint/performance/noDelete: TOML key removal needs literal delete; alternatives (assigning undefined) leak the key during re-serialize.
  delete doc.lints;

  if (doc.package && typeof doc.package === "object") {
    doc.package = flattenPackage(doc.package as TomlTable, ws.pkg);
  }

  for (const key of ["dependencies", "dev-dependencies", "build-dependencies"] as const) {
    const section = doc[key];
    if (section && typeof section === "object" && !Array.isArray(section)) {
      doc[key] = flattenDeps(section as TomlTable, ws.deps);
    }
  }

  // [target.<cfg>.dependencies] — walk the second level.
  if (doc.target && typeof doc.target === "object" && !Array.isArray(doc.target)) {
    for (const [cfg, cfgVal] of Object.entries(doc.target as TomlTable)) {
      if (!cfgVal || typeof cfgVal !== "object" || Array.isArray(cfgVal)) continue;
      const cfgTable = cfgVal as TomlTable;
      for (const depKey of ["dependencies", "dev-dependencies", "build-dependencies"] as const) {
        const section = cfgTable[depKey];
        if (section && typeof section === "object" && !Array.isArray(section)) {
          cfgTable[depKey] = flattenDeps(section as TomlTable, ws.deps);
        }
      }
      (doc.target as TomlTable)[cfg] = cfgTable;
    }
  }

  return stringify(doc);
}

if (!existsSync(HOST_RS_TOML)) {
  throw new Error(`host-rs not found at ${HOST_RS_TOML}`);
}

const ws = await readWorkspaceValues();
const rawHostToml = await Bun.file(HOST_RS_TOML).text();
const flattened = flattenCargoToml(rawHostToml, ws);

if (existsSync(HOST_SRC_NATIVE)) {
  rmSync(HOST_SRC_NATIVE, { recursive: true, force: true });
}
mkdirSync(HOST_SRC_NATIVE, { recursive: true });

// Source tree — copy verbatim.
cpSync(join(HOST_RS, "src"), join(HOST_SRC_NATIVE, "src"), { recursive: true });

// Flattened Cargo.toml — preserves crate name + features + optional deps.
await Bun.write(join(HOST_SRC_NATIVE, "Cargo.toml"), flattened);

const _ok = relative(ROOT, HOST_SRC_NATIVE);
