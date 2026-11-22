import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import { type ArtifactDigest, digestArtifact } from "./check-reproducible-artifact";
import { DESKTOP_TARGETS } from "./desktop-targets";
import { extractReleaseAsset } from "./package-release-asset";

const COMMIT_PATTERN = /^[a-f0-9]{40}$/;
const SHA256_PATTERN = /^[a-f0-9]{64}$/;
const DEFAULT_MAX_AGE_MS = 7 * 24 * 60 * 60 * 1000;

interface TargetFile {
  target: string;
  relative: string;
}

interface BundlerFile extends TargetFile {
  binary: string;
  asset: string;
}

export const RELEASE_BUNDLER_TARGETS: BundlerFile[] = DESKTOP_TARGETS.map((target) => ({
  target: target.releaseBundlerTarget,
  relative: `bundlers/bundler-evidence-${target.releaseBundlerTarget}`,
  binary: target.releaseBundlerBinary,
  asset: target.releaseBundlerAsset,
}));

interface AttestationSubject {
  path: string;
  sha256: string;
}

interface AttestationEvidence {
  schemaVersion: 1;
  commit: string;
  generatedAt: string;
  subjects: AttestationSubject[];
}

interface ReleaseBundlerEvidence {
  target: string;
  binaryPath: string;
  binarySha256: string;
  binarySize: number;
  archiveBinarySha256: string;
  archiveBinarySize: number;
  assetPath: string;
  assetSha256: string;
  assetSize: number;
  sidecarPath: string;
  sidecarSha256: string;
}

export interface ReleaseProvenanceEvidence {
  schemaVersion: 1;
  commit: string;
  generatedAt: string;
  bundlers: ReleaseBundlerEvidence[];
  attestedSubjects: AttestationSubject[];
}

function readJson<T>(file: string): T {
  return JSON.parse(readFileSync(file, "utf8")) as T;
}

function assertDigest(value: unknown, label: string): asserts value is ArtifactDigest {
  if (
    !value ||
    typeof value !== "object" ||
    !("sha256" in value) ||
    typeof value.sha256 !== "string" ||
    !SHA256_PATTERN.test(value.sha256) ||
    !("size" in value) ||
    !Number.isSafeInteger(value.size) ||
    (value.size as number) < 4_096
  ) {
    throw new Error(`${label} has invalid digest or size`);
  }
}

function verifyReproducibleFile(file: string, snapshot: string, label: string): ArtifactDigest {
  if (!existsSync(file) || !existsSync(snapshot)) {
    throw new Error(`${label} file or reproducibility snapshot is missing`);
  }
  const expected = readJson<unknown>(snapshot);
  assertDigest(expected, `${label} snapshot`);
  const actual = digestArtifact(file);
  if (actual.sha256 !== expected.sha256 || actual.size !== expected.size) {
    throw new Error(`${label} does not match clean-rebuild digest`);
  }
  return actual;
}

function expectedAttestedPaths(): string[] {
  return RELEASE_BUNDLER_TARGETS.flatMap(({ relative, asset }) => [
    `${relative}/${asset}`,
    `${relative}/${asset}.sha256`,
  ]).sort();
}

function validateAttestations(root: string, file: string, commit: string): AttestationSubject[] {
  const evidence = readJson<AttestationEvidence>(file);
  if (evidence.schemaVersion !== 1) throw new Error("attestation evidence schemaVersion must be 1");
  if (evidence.commit !== commit) throw new Error("attestation evidence belongs to another commit");
  if (Number.isNaN(Date.parse(evidence.generatedAt))) {
    throw new Error("attestation evidence has invalid timestamp");
  }
  if (!Array.isArray(evidence.subjects)) throw new Error("attestation subjects must be an array");
  const paths = evidence.subjects.map((subject) => subject.path);
  if (new Set(paths).size !== paths.length) throw new Error("attestation subjects are duplicated");
  if (JSON.stringify([...paths].sort()) !== JSON.stringify(expectedAttestedPaths())) {
    throw new Error("attestation subject matrix is incomplete or contains unknown paths");
  }
  for (const subject of evidence.subjects) {
    if (!SHA256_PATTERN.test(subject.sha256)) {
      throw new Error(`attestation subject ${subject.path} has invalid SHA-256`);
    }
    const actual = digestArtifact(path.join(root, ...subject.path.split("/")));
    if (actual.sha256 !== subject.sha256) {
      throw new Error(`attestation subject ${subject.path} hash mismatch`);
    }
  }
  return [...evidence.subjects].sort((left, right) => left.path.localeCompare(right.path));
}

function verifyBundler(
  root: string,
  { target, relative, binary, asset }: BundlerFile,
): ReleaseBundlerEvidence {
  const directory = path.join(root, ...relative.split("/"));
  const binaryPath = path.join(directory, binary);
  const assetPath = path.join(directory, asset);
  const sidecarPath = `${assetPath}.sha256`;
  const binaryDigest = verifyReproducibleFile(
    binaryPath,
    path.join(directory, "bundler-reproducibility.json"),
    `${target} bundler`,
  );
  const assetDigest = digestArtifact(assetPath);
  if (assetDigest.size < 4_096) throw new Error(`${target} bundler asset is suspiciously small`);
  const sidecar = readFileSync(sidecarPath, "utf8").trim();
  if (sidecar !== `${assetDigest.sha256}  ${asset}`) {
    throw new Error(`${target} bundler checksum sidecar does not match asset`);
  }
  const archived = extractReleaseAsset(asset, readFileSync(assetPath));
  const binaryBytes = readFileSync(binaryPath);
  if (archived.filename !== binary || !Buffer.from(archived.contents).equals(binaryBytes)) {
    throw new Error(`${target} bundler asset does not contain the exact release binary`);
  }
  return {
    target,
    binaryPath: `${relative}/${binary}`,
    binarySha256: binaryDigest.sha256,
    binarySize: binaryDigest.size,
    archiveBinarySha256: binaryDigest.sha256,
    archiveBinarySize: binaryDigest.size,
    assetPath: `${relative}/${asset}`,
    assetSha256: assetDigest.sha256,
    assetSize: assetDigest.size,
    sidecarPath: `${relative}/${asset}.sha256`,
    sidecarSha256: digestArtifact(sidecarPath).sha256,
  };
}

export function buildReleaseProvenanceEvidence(
  root: string,
  commit: string,
  attestationFile: string,
  now = new Date(),
): ReleaseProvenanceEvidence {
  if (!COMMIT_PATTERN.test(commit)) throw new Error("commit must be a full lowercase Git SHA");
  const bundlers = RELEASE_BUNDLER_TARGETS.map((spec) => verifyBundler(root, spec));

  return {
    schemaVersion: 1,
    commit,
    generatedAt: now.toISOString(),
    bundlers,
    attestedSubjects: validateAttestations(root, attestationFile, commit),
  };
}

function validateFreshEnvelope(
  value: unknown,
  expectedCommit: string,
  now: Date,
  maxAgeMs: number,
): string[] {
  if (!value || typeof value !== "object") return ["release provenance evidence must be an object"];
  const evidence = value as Partial<ReleaseProvenanceEvidence>;
  const failures: string[] = [];
  if (evidence.schemaVersion !== 1) failures.push("release provenance schemaVersion must be 1");
  if (evidence.commit !== expectedCommit) {
    failures.push(
      `release provenance commit is ${String(evidence.commit)}, expected ${expectedCommit}`,
    );
  }
  const generatedAt = Date.parse(evidence.generatedAt ?? "");
  if (Number.isNaN(generatedAt)) failures.push("release provenance generatedAt is invalid");
  else if (generatedAt > now.getTime())
    failures.push("release provenance generatedAt is in the future");
  else if (now.getTime() - generatedAt > maxAgeMs) {
    failures.push("release provenance evidence is older than 7 days");
  }
  return failures;
}

function targetRecord(
  row: unknown,
  label: string,
  failures: string[],
): Record<string, unknown> | null {
  if (!row || typeof row !== "object" || !("target" in row) || typeof row.target !== "string") {
    failures.push(`${label} contains an invalid target`);
    return null;
  }
  return row as Record<string, unknown>;
}

function validateTargets(
  rows: unknown,
  expectedTargets: string[],
  label: string,
  failures: string[],
): Map<string, Record<string, unknown>> {
  const result = new Map<string, Record<string, unknown>>();
  if (!Array.isArray(rows)) {
    failures.push(`${label} must be an array`);
    return result;
  }
  for (const value of rows) {
    const row = targetRecord(value, label, failures);
    if (!row) continue;
    const target = row.target as string;
    if (result.has(target)) failures.push(`${label} target ${target} is duplicated`);
    result.set(target, row);
  }
  if (JSON.stringify([...result.keys()].sort()) !== JSON.stringify([...expectedTargets].sort())) {
    failures.push(`${label} target matrix is incomplete or unknown`);
  }
  return result;
}

function validSha256(value: unknown): boolean {
  return typeof value === "string" && SHA256_PATTERN.test(value);
}

function validArtifactSize(value: unknown): boolean {
  return Number.isSafeInteger(value) && (value as number) >= 4_096;
}

function validBundlerRow(row: Record<string, unknown>, spec: BundlerFile): boolean {
  if (row.binaryPath !== `${spec.relative}/${spec.binary}`) return false;
  if (row.assetPath !== `${spec.relative}/${spec.asset}`) return false;
  if (row.sidecarPath !== `${spec.relative}/${spec.asset}.sha256`) return false;
  if (!validSha256(row.binarySha256)) return false;
  if (!validSha256(row.assetSha256)) return false;
  if (!validSha256(row.sidecarSha256)) return false;
  if (row.archiveBinarySha256 !== row.binarySha256) return false;
  if (!validArtifactSize(row.binarySize)) return false;
  if (row.archiveBinarySize !== row.binarySize) return false;
  return validArtifactSize(row.assetSize);
}

function validateBundlerRows(
  evidence: Partial<ReleaseProvenanceEvidence>,
  failures: string[],
): Map<string, Record<string, unknown>> {
  const bundlers = validateTargets(
    evidence.bundlers,
    RELEASE_BUNDLER_TARGETS.map(({ target }) => target),
    "release bundlers",
    failures,
  );
  for (const spec of RELEASE_BUNDLER_TARGETS) {
    const row = bundlers.get(spec.target);
    if (row && !validBundlerRow(row, spec)) {
      failures.push(`${spec.target}: invalid bundler provenance record`);
    }
  }
  return bundlers;
}

function isAttestationSubject(value: unknown): value is AttestationSubject {
  return (
    typeof value === "object" &&
    value !== null &&
    "path" in value &&
    typeof value.path === "string" &&
    "sha256" in value &&
    validSha256(value.sha256)
  );
}

function collectAttestedPaths(rows: unknown[], failures: string[]): Map<string, string> {
  const paths = new Map<string, string>();
  for (const value of rows) {
    if (!isAttestationSubject(value)) {
      failures.push("release attestedSubjects contains an invalid subject");
      continue;
    }
    if (paths.has(value.path)) failures.push(`release attestation ${value.path} is duplicated`);
    paths.set(value.path, value.sha256);
  }
  return paths;
}

function validateAttestedHashes(
  paths: Map<string, string>,
  bundlers: Map<string, Record<string, unknown>>,
  failures: string[],
): void {
  for (const bundler of RELEASE_BUNDLER_TARGETS) {
    const row = bundlers.get(bundler.target);
    if (!row) continue;
    if (paths.get(`${bundler.relative}/${bundler.asset}`) !== row.assetSha256) {
      failures.push(`${bundler.target}: attested asset hash does not match`);
    }
    if (paths.get(`${bundler.relative}/${bundler.asset}.sha256`) !== row.sidecarSha256) {
      failures.push(`${bundler.target}: attested sidecar hash does not match`);
    }
  }
}

function validateAttestedSubjects(
  rows: unknown,
  bundlers: Map<string, Record<string, unknown>>,
  failures: string[],
): void {
  if (!Array.isArray(rows)) {
    failures.push("release attestedSubjects must be an array");
    return;
  }
  const paths = collectAttestedPaths(rows, failures);
  if (JSON.stringify([...paths.keys()].sort()) !== JSON.stringify(expectedAttestedPaths())) {
    failures.push("release attestation matrix is incomplete or unknown");
  }
  validateAttestedHashes(paths, bundlers, failures);
}

export function validateReleaseProvenanceMatrix(
  value: unknown,
  expectedCommit: string,
  now = new Date(),
  maxAgeMs = DEFAULT_MAX_AGE_MS,
): string[] {
  const failures = validateFreshEnvelope(value, expectedCommit, now, maxAgeMs);
  if (!value || typeof value !== "object") return failures;
  const evidence = value as Partial<ReleaseProvenanceEvidence>;
  const bundlers = validateBundlerRows(evidence, failures);
  validateAttestedSubjects(evidence.attestedSubjects, bundlers, failures);
  return [...new Set(failures)].sort();
}

function values(args: string[]): Map<string, string> {
  const result = new Map<string, string>();
  for (let index = 0; index < args.length; index += 2) {
    const key = args[index];
    const value = args[index + 1];
    if (!key?.startsWith("--") || !value) throw new Error("invalid release evidence argument");
    result.set(key, value);
  }
  return result;
}

function main(): void {
  const args = values(process.argv.slice(2));
  const commit = args.get("--commit");
  if (!commit || !COMMIT_PATTERN.test(commit)) throw new Error("--commit is required");
  const matrixFile = args.get("--matrix");
  if (matrixFile) {
    if (args.size !== 2) throw new Error("verify mode accepts only --matrix and --commit");
    const failures = validateReleaseProvenanceMatrix(readJson(matrixFile), commit);
    if (failures.length > 0) throw new Error(failures.join("\n"));
    process.stdout.write("Release provenance matrix valid.\n");
    return;
  }
  const root = args.get("--root");
  const attestations = args.get("--attestations");
  const output = args.get("--output");
  if (!root || !attestations || !output || args.size !== 4) {
    throw new Error("build mode requires --root, --attestations, --commit, and --output");
  }
  const evidence = buildReleaseProvenanceEvidence(
    path.resolve(root),
    commit,
    path.resolve(attestations),
  );
  const outputPath = path.resolve(output);
  mkdirSync(path.dirname(outputPath), { recursive: true });
  writeFileSync(outputPath, `${JSON.stringify(evidence, null, 2)}\n`);
  process.stdout.write(
    `Release provenance evidence valid: ${evidence.bundlers.length} bundlers, ${evidence.attestedSubjects.length} attestations.\n`,
  );
}

if (import.meta.main) main();
