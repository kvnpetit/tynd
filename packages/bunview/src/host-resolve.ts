/**
 * Binary resolution utilities for the webview host.
 *
 * Extracted from host.ts to keep binary-finding/extraction logic separate
 * from the WebviewHost class IPC management.
 */

import path from "path";
import os from "os";
import { mkdirSync, existsSync, writeFileSync, chmodSync } from "fs";

/** Returns the platform-specific binary subdirectory name, or null if unsupported. */
export function currentBinDir(): string | null {
  const arch = process.arch === "arm64" ? "arm64" : "x64";
  if (process.platform === "win32")  return `windows-${arch}`;
  if (process.platform === "darwin") return `macos-${arch}`;
  if (process.platform === "linux")  return `linux-${arch}`;
  return null;
}

/** Extract the embedded webview-host binary to a temp dir (cached by size). */
export async function extractEmbeddedHostAsync(
  embeddedPath: string,
  ext: string,
): Promise<string | null> {
  try {
    const embeddedFile = Bun.file(embeddedPath);
    const embeddedSize = embeddedFile.size;

    // Cache dir includes the size as a simple version key — if the binary
    // changes between app versions, a new dir is used automatically.
    const cacheDir = path.join(os.tmpdir(), `bunview-host-${embeddedSize}`);
    const dest     = path.join(cacheDir, `webview-host${ext}`);

    if (!existsSync(dest) || Bun.file(dest).size !== embeddedSize) {
      mkdirSync(cacheDir, { recursive: true });
      const data = await embeddedFile.arrayBuffer();
      writeFileSync(dest, Buffer.from(data));
      if (process.platform !== "win32") chmodSync(dest, 0o755);
    }

    return dest;
  } catch (err) {
    console.error(`[bunview] Failed to extract embedded webview-host: ${err}`);
    return null;
  }
}

/**
 * Resolve the path to the native webview-host binary.
 *
 * Search order:
 *   0. Embedded mode — extract from compiled binary to temp dir
 *   1. Next to the running executable (distributed alongside)
 *   2. Relative to the caller's source dir (dev mode — inside the bunview package)
 *
 * @param callerDir - The directory of the calling module (typically `import.meta.dir`
 *                    from host.ts) used as a base for dev-mode binary lookup.
 */
export async function resolveBinary(callerDir: string): Promise<string | null> {
  const ext = process.platform === "win32" ? ".exe" : "";

  // 0. Embedded mode — extract from compiled binary to temp dir
  const embeddedHost: string | undefined = (globalThis as any).__BUNVIEW_EMBEDDED__?.host;
  if (embeddedHost) {
    return extractEmbeddedHostAsync(embeddedHost, ext);
  }

  const dir = currentBinDir();
  if (!dir) return null;

  // 1. Next to the running executable (distributed alongside)
  // 2. Relative to the caller's source file (dev mode — inside the bunview package)
  const candidates = [
    path.join(path.dirname(process.execPath), "bin", dir, `webview-host${ext}`),
    path.join(callerDir, "../bin", dir, `webview-host${ext}`),
  ];

  for (const file of candidates) {
    try {
      if (Bun.file(file).size > 0) return file;
    } catch { /* not found */ }
  }

  return null;
}
