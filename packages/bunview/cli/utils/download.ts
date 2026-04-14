import { fileURLToPath } from "url";
import { mkdirSync, createWriteStream } from "fs";
import path from "path";
import type { Target } from "./types";
import { log } from "./colors";

function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1048576) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / 1048576).toFixed(1)} MB`;
}

function printProgress(loaded: number, total: number | null, startTime: number): void {
  if (!process.stdout.isTTY) return;
  const elapsed = (Date.now() - startTime) / 1000;
  const speed   = elapsed > 0 ? loaded / elapsed : 0;
  const pct     = total ? Math.round((loaded / total) * 100) : null;
  const bar     = total ? buildBar(loaded / total, 30) : "";
  const msg = pct !== null
    ? `${bar} ${pct}%  ${formatBytes(loaded)}/${formatBytes(total!)}  (${formatBytes(speed)}/s)`
    : `${formatBytes(loaded)}  (${formatBytes(speed)}/s)`;
  process.stdout.write(`\r  ${msg}\x1b[K`);
}

function buildBar(ratio: number, width: number): string {
  const filled = Math.round(ratio * width);
  return `[${"█".repeat(filled)}${" ".repeat(width - filled)}]`;
}

/** Download a webview-host binary from GitHub Releases into the package cache. */
export async function downloadHost(target: Target, destPath: string, ext: string): Promise<void> {
  const bunviewPkg = fileURLToPath(new URL("../../package.json", import.meta.url));
  const pkg = await Bun.file(bunviewPkg).json() as { repository?: { url?: string } };
  const repoUrl   = pkg.repository?.url ?? "";
  const m = repoUrl.match(/github\.com[/:]([\w.-]+\/[\w.-]+?)(?:\.git)?$/);
  if (!m) {
    log.warn("No repository URL in package.json — cannot download webview-host.");
    return;
  }

  const ownerRepo  = m[1];
  const assetName  = `webview-host-${target}${ext}`;
  const url        = `https://github.com/${ownerRepo}/releases/latest/download/${assetName}`;

  log.info(`Downloading webview-host (${target})…`);
  try {
    const res = await fetch(url);
    if (!res.ok) throw new Error(`HTTP ${res.status} ${res.statusText}`);

    mkdirSync(path.dirname(destPath), { recursive: true });

    const total = res.headers.get("content-length");
    const totalBytes = total ? parseInt(total, 10) : null;
    const reader = res.body?.getReader();
    if (!reader) throw new Error("response has no body");

    const stream = createWriteStream(destPath);
    let loaded = 0;
    const start = Date.now();

    try {
      while (true) {
        const { done, value } = await reader.read();
        if (done) break;
        loaded += value.byteLength;
        stream.write(value);
        printProgress(loaded, totalBytes, start);
      }
    } finally {
      stream.end();
      await new Promise<void>((r) => stream.once("close", () => r()));
    }
    if (process.stdout.isTTY) process.stdout.write("\n");

    if (process.platform !== "win32") {
      const { chmodSync } = await import("fs");
      chmodSync(destPath, 0o755);
    }
    log.ok(`Downloaded webview-host (${target}) — ${formatBytes(loaded)}`);
  } catch (err: unknown) {
    const msg = err instanceof Error ? err.message : String(err);
    log.error(`Download failed: ${msg}`);
    log.warn("Fallback: build from source with `BUNVIEW_BUILD_FROM_SOURCE=1 bun install`");
  }
}
