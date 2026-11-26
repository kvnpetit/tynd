import path from "node:path";

interface PackInfo {
  integrity: string;
  name: string;
  version: string;
}

interface PackageManifest {
  version?: unknown;
  publishConfig?: {
    tag?: unknown;
  };
}

export type PublishDecision = "publish" | "already-published";

export function decidePublish(local: PackInfo, remoteIntegrity: string | null): PublishDecision {
  if (!remoteIntegrity) return "publish";
  if (remoteIntegrity !== local.integrity) {
    throw new Error(
      `${local.name}@${local.version} already exists with different integrity\n` +
        `registry: ${remoteIntegrity}\nlocal:    ${local.integrity}`,
    );
  }
  return "already-published";
}

/**
 * npm dist-tag for a package: its `publishConfig.tag`, else `next` for a
 * prerelease version (npm refuses to publish one without a tag, and it must
 * never become `latest`), else `latest`.
 */
export function resolvePublishTag(manifest: PackageManifest): string {
  const tag = manifest.publishConfig?.tag;
  if (tag === undefined) {
    const version = typeof manifest.version === "string" ? manifest.version : "";
    return /^\d+\.\d+\.\d+-/.test(version) ? "next" : "latest";
  }
  if (typeof tag !== "string" || !/^[a-zA-Z][a-zA-Z0-9._-]*$/.test(tag)) {
    throw new Error("publishConfig.tag must be a valid non-semver npm dist-tag");
  }
  return tag;
}

async function capture(
  command: string[],
  cwd: string,
): Promise<{
  exitCode: number;
  stderr: string;
  stdout: string;
}> {
  const processHandle = Bun.spawn(command, {
    cwd,
    stdin: "ignore",
    stdout: "pipe",
    stderr: "pipe",
  });
  const [stdout, stderr, exitCode] = await Promise.all([
    new Response(processHandle.stdout).text(),
    new Response(processHandle.stderr).text(),
    processHandle.exited,
  ]);
  return { exitCode, stdout, stderr };
}

async function localPackInfo(packageDir: string): Promise<PackInfo> {
  const result = await capture(["npm", "pack", "--dry-run", "--json"], packageDir);
  if (result.exitCode !== 0) throw new Error(`npm pack failed:\n${result.stderr}`);
  const pack = JSON.parse(result.stdout) as PackInfo[];
  const info = pack[0];
  if (!info?.name || !info.version || !info.integrity) {
    throw new Error("npm pack did not return name, version, and integrity");
  }
  return info;
}

async function registryIntegrity(info: PackInfo, packageDir: string): Promise<string | null> {
  const result = await capture(
    ["npm", "view", `${info.name}@${info.version}`, "dist.integrity", "--json"],
    packageDir,
  );
  if (result.exitCode === 0) {
    const value = JSON.parse(result.stdout) as unknown;
    if (typeof value !== "string" || value.length === 0) {
      throw new Error(`registry returned invalid integrity for ${info.name}@${info.version}`);
    }
    return value;
  }
  if (/\bE404\b|404 Not Found/i.test(result.stderr)) return null;
  throw new Error(`npm view failed for ${info.name}@${info.version}:\n${result.stderr}`);
}

async function publish(packageDir: string, info: PackInfo, tag: string): Promise<void> {
  let delayMs = 5_000;
  for (let attempt = 1; attempt <= 3; attempt += 1) {
    const processHandle = Bun.spawn(
      ["npm", "publish", "--access", "public", "--provenance", "--tag", tag],
      {
        cwd: packageDir,
        stdin: "inherit",
        stdout: "inherit",
        stderr: "inherit",
      },
    );
    if ((await processHandle.exited) === 0) return;

    const remote = await registryIntegrity(info, packageDir);
    if (remote === info.integrity) {
      process.stdout.write(
        `${info.name}@${info.version} reached the registry despite the client error.\n`,
      );
      return;
    }
    if (attempt === 3) throw new Error(`npm publish failed after ${attempt} attempts`);
    await Bun.sleep(delayMs);
    delayMs *= 2;
  }
}

async function main(): Promise<void> {
  // `--print-tag <dir>` only prints the dist-tag (used by the dry-run steps).
  const printTagOnly = Bun.argv[2] === "--print-tag";
  const packageDir = path.resolve(Bun.argv[printTagOnly ? 3 : 2] ?? ".");
  const manifest = (await Bun.file(
    path.join(packageDir, "package.json"),
  ).json()) as PackageManifest;
  const tag = resolvePublishTag(manifest);
  if (printTagOnly) {
    process.stdout.write(tag);
    return;
  }
  const info = await localPackInfo(packageDir);
  const decision = decidePublish(info, await registryIntegrity(info, packageDir));
  if (decision === "already-published") {
    process.stdout.write(
      `${info.name}@${info.version} is already published with identical integrity; skipping.\n`,
    );
    return;
  }
  await publish(packageDir, info, tag);
}

if (import.meta.main) await main();
