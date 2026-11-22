import { cpSync, existsSync, mkdirSync, readdirSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, relative, resolve } from "node:path";
import { parse, stringify } from "smol-toml";
import {
  type DepRewrite,
  flattenDeps,
  mapDepSections,
  mapTargetDepSections,
  parseStandaloneManifest,
  pinPath,
  ROOT,
  readWorkspaceValues,
  type TomlTable,
  vendorCrate,
  type WorkspaceValues,
} from "./mirror";

const HOST_RS = join(ROOT, "crates", "host");
const RUNTIME_RS = join(ROOT, "crates", "runtime");
const DEFAULT_HOST_SRC_NATIVE = join(import.meta.dir, "..", "native");
const outputFlag = Bun.argv.indexOf("--output");
const requestedOutput = outputFlag >= 0 ? Bun.argv[outputFlag + 1] : undefined;
if (outputFlag >= 0 && !requestedOutput) {
  throw new Error("--output requires a path");
}
const HOST_SRC_NATIVE = requestedOutput ? resolve(requestedOutput) : DEFAULT_HOST_SRC_NATIVE;
if (requestedOutput) {
  const relativeToTemp = relative(resolve(tmpdir()), HOST_SRC_NATIVE);
  if (
    relativeToTemp === "" ||
    relativeToTemp === ".." ||
    relativeToTemp.startsWith(`..${process.platform === "win32" ? "\\" : "/"}`)
  ) {
    throw new Error("--output must resolve inside the operating-system temporary directory");
  }
}
const HOST_SRC_LOCK = join(HOST_SRC_NATIVE, "Cargo.lock");
const ROOT_LOCK = join(ROOT, "Cargo.lock");
const VENDOR_DIR = join(HOST_SRC_NATIVE, "vendor");
const HOST_RS_TOML = join(HOST_RS, "Cargo.toml");

const AUX_FILES = ["build.rs", "Info.plist", "README.md", "NOTICE", "LICENSE"];

interface VendorTarget {
  key: string;
  src: string;
  vendorRel: string;
}

/** A vendored crate keeps its package name as its directory under `vendor/`. */
function vendored(key: string, src: string): VendorTarget {
  return { key, src, vendorRel: key };
}

const crate = (dir: string) => join(ROOT, "crates", dir);

const VENDOR_TARGETS: VendorTarget[] = [
  vendored("tynd-plugin-sdk", crate("plugin-sdk")),
  vendored("tynd-host-macros", crate("host-macros")),
  vendored("tynd-capabilities", crate("capabilities")),
  vendored("tynd-debug-protocol", crate("debug-protocol")),
  vendored("tynd-pack-format", crate("pack-format")),
  vendored("tynd-protocol", crate("protocol")),
  vendored("tynd-security-policy", crate("security-policy")),
  vendored("tynd-update-core", crate("update-core")),
];

interface TransitiveVendor {
  vendorRel: string;
  depKey: string;
  src: string;
  rewriteTo: string;
  copyTo: string;
}

const TRANSITIVE_VENDORS: TransitiveVendor[] = [
  {
    vendorRel: "tynd-plugin-sdk",
    depKey: "tynd-plugin-macros",
    src: crate("plugin-macros"),
    rewriteTo: "../tynd-plugin-macros",
    copyTo: "tynd-plugin-macros",
  },
];

function rewriteVendoredDeps(deps: TomlTable, vendorBase: string): TomlTable {
  const out: TomlTable = {};
  for (const [k, v] of Object.entries(deps)) {
    const target = VENDOR_TARGETS.find((t) => t.key === k);
    out[k] = target ? pinPath(v, `${vendorBase}/${target.vendorRel}`) : v;
  }
  return out;
}

function flattenCargoToml(
  src: string,
  ws: WorkspaceValues,
  vendorBase = "vendor",
  hostPath?: string,
): string {
  const doc = parseStandaloneManifest(src, ws);
  const flattenSection: DepRewrite = (section) =>
    rewriteVendoredDeps(flattenDeps(section, ws.deps), vendorBase);
  mapDepSections(doc, (section) => {
    const flattened = flattenSection(section);
    if (hostPath && "tynd-host" in flattened) {
      flattened["tynd-host"] = pinPath(flattened["tynd-host"], hostPath);
    }
    return flattened;
  });
  mapTargetDepSections(doc, flattenSection);
  return stringify(doc);
}

function assertSourcesExist(): void {
  if (!existsSync(HOST_RS_TOML)) {
    throw new Error(`host crate not found at ${HOST_RS_TOML}`);
  }
  if (!existsSync(ROOT_LOCK)) {
    throw new Error(`workspace lockfile not found at ${ROOT_LOCK}`);
  }
  for (const target of VENDOR_TARGETS) {
    if (!existsSync(join(target.src, "Cargo.toml"))) {
      throw new Error(`vendor source missing: ${target.key} → ${target.src}`);
    }
  }
  for (const t of TRANSITIVE_VENDORS) {
    if (!existsSync(join(t.src, "Cargo.toml"))) {
      throw new Error(`transitive vendor source missing: ${t.depKey} → ${t.src}`);
    }
  }
}

/** Clear the generated mirror but keep `target/`, so builds of the mirror stay incremental. */
function clearMirror(): void {
  if (existsSync(HOST_SRC_NATIVE)) {
    for (const entry of readdirSync(HOST_SRC_NATIVE)) {
      if (entry !== "target") {
        rmSync(join(HOST_SRC_NATIVE, entry), { recursive: true, force: true });
      }
    }
  }
  mkdirSync(HOST_SRC_NATIVE, { recursive: true });
}

function copyHostSources(): void {
  cpSync(join(HOST_RS, "src"), join(HOST_SRC_NATIVE, "src"), { recursive: true });
  cpSync(join(RUNTIME_RS, "src"), join(HOST_SRC_NATIVE, "runtime", "src"), { recursive: true });
  for (const f of AUX_FILES) {
    const src = join(HOST_RS, f);
    if (existsSync(src)) {
      cpSync(src, join(HOST_SRC_NATIVE, f));
    }
  }
}

function lockedPackages(lockPath: string): Set<string> {
  const doc = parse(readFileSync(lockPath, "utf8")) as TomlTable;
  const packages = Array.isArray(doc.package) ? (doc.package as TomlTable[]) : [];
  return new Set(
    packages
      .filter((p) => typeof p.source === "string")
      .map((p) => `${p.name} ${p.version} ${p.source} ${p.checksum ?? ""}`),
  );
}

// The mirror lock is derived, never hand-maintained: seed it with the root
// workspace lock and let Cargo prune it to the mirror's dependency graph.
// `cargo update --workspace` keeps every already-locked version, so the result
// is the root lock's pins minus the packages the mirror does not use, and it
// is exactly what `--locked` accepts for the manifests written by this script.
function deriveLock(): void {
  cpSync(ROOT_LOCK, HOST_SRC_LOCK);
  const prune = Bun.spawnSync(
    ["cargo", "update", "--workspace", "--manifest-path", join(HOST_SRC_NATIVE, "Cargo.toml")],
    { stdout: "ignore", stderr: "pipe" },
  );
  if (prune.exitCode !== 0) {
    throw new Error(`cargo failed to derive ${HOST_SRC_LOCK}: ${prune.stderr.toString()}`);
  }
  const rootPins = lockedPackages(ROOT_LOCK);
  const unpinned = [...lockedPackages(HOST_SRC_LOCK)].filter((p) => !rootPins.has(p));
  if (unpinned.length > 0) {
    throw new Error(
      `host-src lock resolved packages the root Cargo.lock does not pin: ${unpinned.join(", ")}`,
    );
  }
}

assertSourcesExist();

const ws = await readWorkspaceValues();
const rawHostToml = await Bun.file(HOST_RS_TOML).text();
const flattenedHost = flattenCargoToml(rawHostToml, ws);
const rawRuntimeToml = await Bun.file(join(RUNTIME_RS, "Cargo.toml")).text();
const flattenedRuntime = flattenCargoToml(rawRuntimeToml, ws, "../vendor", "..");

clearMirror();
copyHostSources();

mkdirSync(VENDOR_DIR, { recursive: true });
for (const target of VENDOR_TARGETS) {
  const siblingPaths = Object.fromEntries(
    TRANSITIVE_VENDORS.filter((t) => t.vendorRel === target.vendorRel).map((t) => [
      t.depKey,
      t.rewriteTo,
    ]),
  );
  await vendorCrate(target.src, join(VENDOR_DIR, target.vendorRel), ws, { siblingPaths });
}
for (const t of TRANSITIVE_VENDORS) {
  await vendorCrate(t.src, join(VENDOR_DIR, t.copyTo), ws);
}

const generatedWorkspace = stringify({
  workspace: { resolver: "2", members: ["runtime"], lints: ws.lints },
});
await Bun.write(join(HOST_SRC_NATIVE, "Cargo.toml"), `${flattenedHost}\n\n${generatedWorkspace}\n`);
await Bun.write(join(HOST_SRC_NATIVE, "runtime", "Cargo.toml"), flattenedRuntime);

deriveLock();
