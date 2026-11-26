import { createHash } from "node:crypto";
import { existsSync, readFileSync, statSync, writeFileSync } from "node:fs";
import path from "node:path";

export interface ArtifactDigest {
  sha256: string;
  size: number;
}

export function digestArtifact(file: string): ArtifactDigest {
  if (!existsSync(file) || !statSync(file).isFile()) {
    throw new Error(`artifact does not exist or is not a file: ${file}`);
  }
  const bytes = readFileSync(file);
  return {
    sha256: createHash("sha256").update(bytes).digest("hex"),
    size: bytes.byteLength,
  };
}

export function recordArtifact(file: string, snapshot: string): ArtifactDigest {
  const digest = digestArtifact(file);
  writeFileSync(snapshot, `${JSON.stringify(digest, null, 2)}\n`);
  return digest;
}

export function verifyArtifact(file: string, snapshot: string): ArtifactDigest {
  if (!existsSync(snapshot)) throw new Error(`digest snapshot does not exist: ${snapshot}`);
  const expected = JSON.parse(readFileSync(snapshot, "utf8")) as Partial<ArtifactDigest>;
  if (
    typeof expected.sha256 !== "string" ||
    !/^[a-f0-9]{64}$/.test(expected.sha256) ||
    !Number.isSafeInteger(expected.size) ||
    (expected.size ?? -1) < 0
  ) {
    throw new Error(`invalid artifact digest snapshot: ${snapshot}`);
  }

  const actual = digestArtifact(file);
  if (actual.sha256 !== expected.sha256 || actual.size !== expected.size) {
    throw new Error(
      `non-reproducible artifact ${file}: expected ${expected.sha256}/${expected.size} bytes, ` +
        `got ${actual.sha256}/${actual.size} bytes`,
    );
  }
  return actual;
}

function usage(): never {
  throw new Error(
    "usage: check-reproducible-artifact.ts <record|verify> <artifact> <digest-snapshot>",
  );
}

function main(): void {
  const [operation, artifactArg, snapshotArg, ...extra] = process.argv.slice(2);
  if (!operation || !artifactArg || !snapshotArg || extra.length > 0) usage();
  const artifact = path.resolve(artifactArg);
  const snapshot = path.resolve(snapshotArg);

  const digest =
    operation === "record"
      ? recordArtifact(artifact, snapshot)
      : operation === "verify"
        ? verifyArtifact(artifact, snapshot)
        : usage();
  process.stdout.write(`${operation}: ${artifact} ${digest.sha256} (${digest.size} bytes)\n`);
}

if (import.meta.main) main();
