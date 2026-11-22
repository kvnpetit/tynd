import { spawn } from "node:child_process";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const SUPPORTED_PLATFORMS = new Set(["darwin", "linux", "win32"]);
const SUPPORTED_ARCHITECTURES = new Set(["arm64", "x64"]);
const SUPPORTED_BINARIES = new Set(["tynd", "tynd-mcp", "tynd-bundler"]);

export function toolingSubPackageName(platform = process.platform, arch = process.arch) {
  if (!SUPPORTED_PLATFORMS.has(platform) || !SUPPORTED_ARCHITECTURES.has(arch)) {
    throw new Error(
      `Unsupported Tynd tooling target: ${platform}-${arch}. Supported targets: darwin-arm64, darwin-x64, linux-arm64, linux-x64, win32-arm64, win32-x64.`,
    );
  }
  return `@tynd/tooling-${platform}-${arch}`;
}

export function resolveToolingBinary(
  binary,
  { platform = process.platform, arch = process.arch } = {},
) {
  if (!SUPPORTED_BINARIES.has(binary)) {
    throw new Error(
      `Unknown Tynd tooling binary "${binary}". Expected tynd, tynd-mcp, or tynd-bundler.`,
    );
  }

  const packageName = toolingSubPackageName(platform, arch);
  const extension = platform === "win32" ? ".exe" : "";
  try {
    return require.resolve(`${packageName}/bin/${binary}${extension}`);
  } catch (cause) {
    throw new Error(
      `Could not resolve ${binary} for ${platform}-${arch}. Reinstall the package without disabling optional dependencies.`,
      { cause },
    );
  }
}

export function runToolingBinary(binary, args = process.argv.slice(2)) {
  const executable = resolveToolingBinary(binary);
  return new Promise((resolve, reject) => {
    const child = spawn(executable, args, {
      stdio: "inherit",
      windowsHide: false,
    });
    child.once("error", reject);
    child.once("exit", (code, signal) => {
      if (signal) {
        try {
          process.kill(process.pid, signal);
        } catch {
          resolve(1);
        }
        return;
      }
      resolve(code ?? 1);
    });
  });
}
