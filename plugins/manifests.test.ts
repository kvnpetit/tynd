import { describe, expect, test } from "bun:test";
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";

type Kind = "host" | "standalone";

interface Manifest {
  name?: string;
  homepage?: string;
  dependencies?: Record<string, string>;
  devDependencies?: Record<string, string>;
  optionalDependencies?: Record<string, string>;
  peerDependencies?: Record<string, string>;
  peerDependenciesMeta?: Record<string, { optional?: boolean }>;
  files?: string[];
  exports?: Record<string, unknown> | string;
  tynd?: { kind?: string; [key: string]: unknown };
}

const pluginsRoot = import.meta.dir;
const HOMEPAGE_BASE = "https://tynd-apps.github.io/tynd/docs/v1.0/api/plugins/";

const plugins = (["host", "standalone"] as const).flatMap((kind: Kind) =>
  readdirSync(join(pluginsRoot, kind), { withFileTypes: true })
    .filter(
      (entry) =>
        entry.isDirectory() && existsSync(join(pluginsRoot, kind, entry.name, "package.json")),
    )
    .map((entry) => ({
      kind,
      dir: entry.name,
      manifest: JSON.parse(
        readFileSync(join(pluginsRoot, kind, entry.name, "package.json"), "utf8"),
      ) as Manifest,
    })),
);

function exportTargets(value: unknown): string[] {
  if (typeof value === "string") return value.startsWith("./") ? [value] : [];
  if (value && typeof value === "object") return Object.values(value).flatMap(exportTargets);
  return [];
}

test("plugin manifests are discovered", () => {
  expect(plugins.filter((p) => p.kind === "host").length).toBeGreaterThan(0);
  expect(plugins.filter((p) => p.kind === "standalone").length).toBeGreaterThan(0);
});

describe.each(plugins)("$kind/$dir", ({ kind, dir, manifest }) => {
  test("name follows @tynd/plugin-<dir>", () => {
    expect(manifest.name).toBe(`@tynd/plugin-${dir}`);
  });

  test("peer dependencies on @tynd/* match the plugin kind", () => {
    const peers = Object.keys(manifest.peerDependencies ?? {}).filter((n) =>
      n.startsWith("@tynd/"),
    );
    expect(peers.length).toBeGreaterThan(0);
    for (const peer of peers) {
      if (kind === "host") {
        // Host plugin clients build on `@tynd/core/client/bridge`: core is mandatory.
        expect(manifest.peerDependenciesMeta?.[peer]?.optional).not.toBe(true);
      } else {
        expect(manifest.peerDependenciesMeta?.[peer]?.optional).toBe(true);
      }
    }
  });

  test.skipIf(kind !== "host")("host-backed plugins require @tynd/core >=1.0.0 <2", () => {
    expect(manifest.peerDependencies?.["@tynd/core"]).toBe(">=1.0.0 <2");
  });

  test("homepage points to the plugin docs page", () => {
    expect(manifest.homepage).toBe(`${HOMEPAGE_BASE}${dir}`);
  });

  test("@tynd/* installed dependencies use workspace: ranges", () => {
    for (const field of ["dependencies", "devDependencies", "optionalDependencies"] as const) {
      for (const [dep, range] of Object.entries(manifest[field] ?? {})) {
        if (dep.startsWith("@tynd/")) {
          expect(`${field}:${dep}:${range}`).toMatch(new RegExp(`^${field}:${dep}:workspace:`));
        }
      }
    }
  });

  test("every files entry exists and LICENSE and NOTICE ship", () => {
    const root = join(pluginsRoot, kind, dir);
    expect(manifest.files).toContain("LICENSE");
    expect(manifest.files).toContain("NOTICE");
    const missing = (manifest.files ?? []).filter(
      (entry) => !existsSync(join(root, entry.replace(/\/\*\*$/, ""))),
    );
    expect(missing).toEqual([]);
  });

  test("every exports target exists and is listed in files", () => {
    const root = join(pluginsRoot, kind, dir);
    const targets = exportTargets(manifest.exports).map((target) => target.replace(/^\.\//, ""));
    expect(targets.length).toBeGreaterThan(0);
    expect(targets.filter((file) => !existsSync(join(root, file)))).toEqual([]);
    expect(targets.filter((file) => !manifest.files?.includes(file))).toEqual([]);
  });

  test("tynd field declares the plugin kind", () => {
    expect(manifest.tynd).toBeDefined();
    expect(manifest.tynd?.kind).toBe(kind);
    // Mirrors the schema `tynd plugin check` enforces (crates/cli validation.rs).
    const allowed = ["kind", "apis", "native", "backend"];
    expect(Object.keys(manifest.tynd ?? {}).filter((key) => !allowed.includes(key))).toEqual([]);
    if (kind === "host") {
      expect(manifest.tynd?.apis).toEqual([dir]);
    }
  });
});
