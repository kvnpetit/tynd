import path from "path";
import { existsSync } from "fs";
import type { BunviewConfig } from "../../src/types";

/**
 * Resolve ${ENV_VAR} placeholders in a string.
 * e.g. "${WINDOWS_CERT_PASSWORD}" → process.env.WINDOWS_CERT_PASSWORD
 */
function resolveEnv(value: string | undefined): string | undefined {
  if (!value) return value;
  return value.replace(/\$\{(\w+)\}/g, (_, name) => process.env[name] ?? "");
}

async function run(cmd: string, args: string[], label: string): Promise<boolean> {
  const proc = Bun.spawn([cmd, ...args], { stdout: "inherit", stderr: "inherit" });
  const code = await proc.exited;
  if (code !== 0) {
    console.error(`[bunview] ❌ ${label} failed (exit ${code})`);
    return false;
  }
  return true;
}

export async function signWindows(
  binary: string,
  config: NonNullable<NonNullable<BunviewConfig["codeSigning"]>["windows"]>,
): Promise<boolean> {
  const cert = path.resolve(process.cwd(), config.certificate);
  if (!existsSync(cert)) {
    console.error(`[bunview] ❌ Windows cert not found: ${cert}`);
    return false;
  }
  const password = resolveEnv(config.password);
  const timestamp = config.timestampUrl ?? "http://timestamp.digicert.com";

  const args = [
    "sign",
    "/f", cert,
    ...(password ? ["/p", password] : []),
    "/tr", timestamp,
    "/td", "sha256",
    "/fd", "sha256",
    ...(config.description ? ["/d", config.description] : []),
    binary,
  ];

  console.log(`[bunview] Signing ${path.basename(binary)} (Windows)...`);
  return run("signtool", args, "signtool sign");
}

export async function signMacos(
  target: string,
  config: NonNullable<NonNullable<BunviewConfig["codeSigning"]>["macos"]>,
): Promise<boolean> {
  const entitlements = config.entitlements
    ? path.resolve(process.cwd(), config.entitlements)
    : undefined;
  if (entitlements && !existsSync(entitlements)) {
    console.error(`[bunview] ❌ Entitlements file not found: ${entitlements}`);
    return false;
  }

  const args = [
    "--force",
    "--sign", config.identity,
    "--timestamp",
    ...(config.hardenedRuntime !== false ? ["--options", "runtime"] : []),
    ...(entitlements ? ["--entitlements", entitlements] : []),
    target,
  ];

  console.log(`[bunview] Signing ${path.basename(target)} (macOS, hardened runtime)...`);
  return run("codesign", args, "codesign");
}

/**
 * Verify a macOS binary/bundle was signed correctly.
 */
export async function verifyMacos(target: string): Promise<boolean> {
  return run("codesign", ["--verify", "--deep", "--strict", "--verbose=2", target], "codesign --verify");
}

/**
 * Submit a signed macOS binary to Apple for notarization and wait for the result.
 * Must be signed with hardened runtime + a Developer ID Application cert.
 */
export async function notarizeMacos(
  target: string,
  config: NonNullable<NonNullable<NonNullable<BunviewConfig["codeSigning"]>["macos"]>["notarize"]>,
): Promise<boolean> {
  const appleId = resolveEnv(config.appleId);
  const teamId  = resolveEnv(config.teamId);
  const password = resolveEnv(config.password);

  if (!appleId || !teamId || !password) {
    console.error(`[bunview] ❌ Notarization requires appleId, teamId, password.`);
    return false;
  }

  // notarytool needs a .zip (or .dmg/.pkg). Create a zip wrapping the target.
  const tmpZip = `${target}.notarize.zip`;
  console.log(`[bunview] Creating zip for notarization...`);
  if (!await run("ditto", ["-c", "-k", "--keepParent", target, tmpZip], "ditto")) return false;

  console.log(`[bunview] Submitting to Apple notary service (may take a few minutes)...`);
  const submitted = await run("xcrun", [
    "notarytool", "submit", tmpZip,
    "--apple-id", appleId,
    "--team-id", teamId,
    "--password", password,
    "--wait",
  ], "notarytool submit");

  // Clean up zip regardless of outcome
  try { await Bun.file(tmpZip).delete?.(); } catch {}

  if (!submitted) return false;

  console.log(`[bunview] Stapling notarization ticket...`);
  return run("xcrun", ["stapler", "staple", target], "stapler staple");
}

/**
 * Sign (and optionally notarize) a built binary according to config.
 * No-op if `config.codeSigning` is unset for the current platform.
 */
export async function signIfConfigured(
  binary: string,
  target: string,
  config: BunviewConfig,
  notarize = true,
): Promise<void> {
  const signing = config.codeSigning;
  if (!signing) return;

  if (target.startsWith("windows") && signing.windows) {
    const ok = await signWindows(binary, signing.windows);
    if (!ok) process.exit(1);
    console.log(`[bunview] ✅ Signed ${path.basename(binary)}`);
    return;
  }

  if (target.startsWith("macos") && signing.macos) {
    const ok = await signMacos(binary, signing.macos);
    if (!ok) process.exit(1);
    console.log(`[bunview] ✅ Signed ${path.basename(binary)}`);

    if (notarize && signing.macos.notarize) {
      const notarized = await notarizeMacos(binary, signing.macos.notarize);
      if (!notarized) {
        console.warn(`[bunview] ⚠  Notarization failed — ${path.basename(binary)} is signed but not stapled.`);
      } else {
        console.log(`[bunview] ✅ Notarized and stapled ${path.basename(binary)}`);
      }
    }
  }
}

/**
 * Sign every signable artifact in `outDir` (main binary, portable, installer, .app, .dmg).
 * Notarization only runs on the final distribution artifacts (.dmg, -installer.exe).
 */
export async function signAllArtifacts(
  outDir: string,
  appName: string,
  target: string,
  config: BunviewConfig,
): Promise<void> {
  if (!config.codeSigning) return;

  const { readdirSync, statSync } = await import("fs");
  const entries = readdirSync(outDir);

  // Patterns that identify signable artifacts
  const signable = entries.filter((name) => {
    // signtool.exe handles both PE binaries (.exe) and MSI packages.
    if (target.startsWith("windows")) return name.endsWith(".exe") || name.endsWith(".msi");
    if (target.startsWith("macos"))   return name.endsWith(".app") || name.endsWith(".dmg");
    // Linux .rpm supports detached GPG signatures via `rpm --addsign` but that
    // requires a separate GPG key and is out of scope for codeSigning config.
    return false;
  });

  for (const name of signable) {
    const fullPath = path.join(outDir, name);
    try { statSync(fullPath); } catch { continue; }
    // Only notarize the distribution artifacts, not intermediate binaries.
    const shouldNotarize =
      target.startsWith("macos") && (name.endsWith(".dmg") || name === `${appName}.app`);
    await signIfConfigured(fullPath, target, config, shouldNotarize);
  }
}
