import { existsSync, readdirSync, readFileSync, statSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const TARGETS = [
  { os: "darwin", arch: "arm64" },
  { os: "darwin", arch: "x64" },
  { os: "linux", arch: "arm64" },
  { os: "linux", arch: "x64" },
  { os: "win32", arch: "arm64" },
  { os: "win32", arch: "x64" },
];
const BINARY_NAMES = ["tynd", "tynd-mcp", "tynd-bundler"];

function readManifest(packageDir) {
  return JSON.parse(readFileSync(path.join(packageDir, "package.json"), "utf8"));
}

function assertSingleValue(value, expected, field, packageName) {
  if (!Array.isArray(value) || value.length !== 1 || value[0] !== expected) {
    throw new Error(`${packageName} must declare ${field}: ["${expected}"]`);
  }
}

function assertPe(binaryPath, bytes, arch) {
  if (bytes[0] !== 0x4d || bytes[1] !== 0x5a) {
    throw new Error(`${binaryPath} is not a PE executable`);
  }
  const peOffset = bytes.readUInt32LE(0x3c);
  if (peOffset + 6 > bytes.length || bytes.toString("ascii", peOffset, peOffset + 4) !== "PE\0\0") {
    throw new Error(`${binaryPath} has an invalid PE header`);
  }
  const expectedMachine = arch === "arm64" ? 0xaa64 : 0x8664;
  if (bytes.readUInt16LE(peOffset + 4) !== expectedMachine) {
    throw new Error(`${binaryPath} PE architecture does not match ${arch}`);
  }
}

function assertElf(binaryPath, bytes, arch) {
  if (!bytes.subarray(0, 4).equals(Buffer.from([0x7f, 0x45, 0x4c, 0x46]))) {
    throw new Error(`${binaryPath} is not an ELF executable`);
  }
  const machine = bytes[5] === 1 ? bytes.readUInt16LE(18) : bytes.readUInt16BE(18);
  const expectedMachine = arch === "arm64" ? 0xb7 : 0x3e;
  if (machine !== expectedMachine) {
    throw new Error(`${binaryPath} ELF architecture does not match ${arch}`);
  }
}

function assertMachO(binaryPath, bytes, arch) {
  const littleEndian = bytes.readUInt32LE(0) === 0xfeedfacf;
  const bigEndian = bytes.readUInt32BE(0) === 0xfeedfacf;
  if (!littleEndian && !bigEndian) {
    throw new Error(`${binaryPath} is not a 64-bit Mach-O executable`);
  }
  const cpuType = littleEndian ? bytes.readUInt32LE(4) : bytes.readUInt32BE(4);
  const expectedCpuType = arch === "arm64" ? 0x0100000c : 0x01000007;
  if (cpuType !== expectedCpuType) {
    throw new Error(`${binaryPath} Mach-O architecture does not match ${arch}`);
  }
}

function assertBinary(binaryPath, target) {
  const bytes = readFileSync(binaryPath);
  if (bytes.length < 64) {
    throw new Error(`${binaryPath} is too small to be a native executable`);
  }
  if (target.os === "win32") {
    assertPe(binaryPath, bytes, target.arch);
  } else if (target.os === "linux") {
    assertElf(binaryPath, bytes, target.arch);
  } else {
    assertMachO(binaryPath, bytes, target.arch);
  }
}

function validateWrapper(repoRoot) {
  const wrapper = readManifest(path.join(repoRoot, "npm", "tooling"));
  if (typeof wrapper.version !== "string") {
    throw new Error("@tynd/tooling needs a string version");
  }
  const dependencies = wrapper.optionalDependencies;
  if (!dependencies || typeof dependencies !== "object" || Array.isArray(dependencies)) {
    throw new Error("@tynd/tooling needs optionalDependencies");
  }
  const expectedNames = new Set(TARGETS.map(({ os, arch }) => `@tynd/tooling-${os}-${arch}`));
  const actualNames = Object.keys(dependencies);
  if (
    actualNames.length !== expectedNames.size ||
    actualNames.some((name) => !expectedNames.has(name))
  ) {
    throw new Error("@tynd/tooling optionalDependencies must exactly match supported targets");
  }
  return { dependencies, version: wrapper.version };
}

function validateManifest(repoRoot, target, wrapper) {
  const packageName = `@tynd/tooling-${target.os}-${target.arch}`;
  const packageDir = path.join(repoRoot, "npm", `tooling-${target.os}-${target.arch}`);
  const manifest = readManifest(packageDir);
  if (manifest.name !== packageName) {
    throw new Error(`${packageDir} must be named ${packageName}`);
  }
  if (
    manifest.version !== wrapper.version ||
    wrapper.dependencies[packageName] !== wrapper.version
  ) {
    throw new Error(`${packageName} version must exactly match @tynd/tooling`);
  }
  if (manifest.private !== false) {
    throw new Error(`${packageName} must be publishable`);
  }
  assertSingleValue(manifest.os, target.os, "os", packageName);
  assertSingleValue(manifest.cpu, target.arch, "cpu", packageName);
  if (!Array.isArray(manifest.files) || !manifest.files.includes("bin/")) {
    throw new Error(`${packageName} must publish bin/`);
  }
  if (manifest.publishConfig?.access !== "public") {
    throw new Error(`${packageName} must publish with public access`);
  }
  return { packageDir, packageName };
}

function validateBinaries(target, packageDir, packageName) {
  const extension = target.os === "win32" ? ".exe" : "";
  const expected = BINARY_NAMES.map((name) => `${name}${extension}`).sort();
  const binDir = path.join(packageDir, "bin");
  const actual = readdirSync(binDir)
    .filter((entry) => entry !== ".gitkeep")
    .sort();
  if (actual.length !== expected.length || actual.some((name, index) => name !== expected[index])) {
    throw new Error(`${packageName} bin/ must contain only ${expected.join(", ")}`);
  }
  for (const binary of actual) {
    const binaryPath = path.join(binDir, binary);
    const stats = statSync(binaryPath);
    if (!stats.isFile()) {
      throw new Error(`${binaryPath} must be a regular file`);
    }
    assertBinary(binaryPath, target);
    if (process.platform !== "win32" && target.os !== "win32" && (stats.mode & 0o111) === 0) {
      throw new Error(`${binaryPath} must be executable`);
    }
  }
  for (const requiredFile of ["LICENSE", "NOTICE", "README.md"]) {
    if (!existsSync(path.join(packageDir, requiredFile))) {
      throw new Error(`${packageName} is missing ${requiredFile}`);
    }
  }
}

export function validateToolingPlatformPackages(repoRoot, { requireBinaries = true } = {}) {
  const wrapper = validateWrapper(repoRoot);
  for (const target of TARGETS) {
    const packageInfo = validateManifest(repoRoot, target, wrapper);
    if (requireBinaries) {
      validateBinaries(target, packageInfo.packageDir, packageInfo.packageName);
    }
  }
  return TARGETS.length;
}

export function validateMcpPackage(repoRoot) {
  const tooling = readManifest(path.join(repoRoot, "npm", "tooling"));
  const mcp = readManifest(path.join(repoRoot, "packages", "mcp"));
  if (mcp.name !== "@tynd/mcp" || mcp.private !== false) {
    throw new Error("@tynd/mcp must be a publishable package");
  }
  if (mcp.version !== tooling.version || mcp.dependencies?.["@tynd/tooling"] !== tooling.version) {
    throw new Error("@tynd/mcp and @tynd/tooling versions must exactly match");
  }
  const dependencyNames = Object.keys(mcp.dependencies ?? {});
  if (dependencyNames.length !== 1 || dependencyNames[0] !== "@tynd/tooling") {
    throw new Error("@tynd/mcp must have @tynd/tooling as its only runtime dependency");
  }
  if (!Array.isArray(mcp.files) || !mcp.files.includes("bin")) {
    throw new Error("@tynd/mcp must publish bin");
  }
  if (mcp.publishConfig?.access !== "public") {
    throw new Error("@tynd/mcp must publish with public access");
  }
}

const metadataOnly = process.argv.includes("--metadata-only");
const rootArgument = process.argv.slice(2).find((argument) => !argument.startsWith("--"));
const defaultRoot = fileURLToPath(new URL("../../..", import.meta.url));
const repoRoot = path.resolve(rootArgument ?? defaultRoot);
const count = validateToolingPlatformPackages(repoRoot, {
  requireBinaries: !metadataOnly,
});
validateMcpPackage(repoRoot);
process.stdout.write(
  `tooling platform packages valid: ${count} (${metadataOnly ? "metadata" : "staged"})\n` +
    "@tynd/mcp package valid\n",
);
