// GitHub Releases-based auto-updater.

import path from "path";
import { writeFileSync, chmodSync, renameSync, unlinkSync } from "fs";

const PLATFORM_PATTERNS: Record<string, string[]> = {
  "windows-x64":   ["windows-x64", "win64", "win-x64"],
  "windows-arm64": ["windows-arm64", "win-arm64"],
  "linux-x64":     ["linux-x64", "linux-amd64"],
  "linux-arm64":   ["linux-arm64", "linux-aarch64"],
  "macos-x64":     ["macos-x64", "darwin-x64", "mac-x64"],
  "macos-arm64":   ["macos-arm64", "darwin-arm64", "darwin-aarch64", "mac-arm64"],
};

export interface UpdateCheckOptions {
  /** GitHub `"owner/name"`. */
  repo: string;
  /** Semver — e.g. `"1.0.0"`. */
  currentVersion: string;
  /** Auto-detected from process.platform + arch if omitted. */
  platform?: string;
}

export interface UpdateInfo {
  version: string;
  downloadUrl: string;
  notes: string;
  date: string;
}

export interface DownloadProgress {
  downloaded: number;
  /** 0 if Content-Length was absent. */
  total: number;
  percent: number;
}

/** Returns null when already up to date. */
export async function check(options: UpdateCheckOptions): Promise<UpdateInfo | null> {
  const { repo, currentVersion } = options;

  try {
    const res = await fetch(`https://api.github.com/repos/${repo}/releases/latest`, {
      headers: { "Accept": "application/vnd.github.v3+json", "User-Agent": "bunview-updater" },
    });
    if (!res.ok) return null;

    const release = await res.json() as {
      tag_name: string;
      body: string;
      published_at: string;
      assets: { name: string; browser_download_url: string }[];
    };

    const remoteVersion = release.tag_name.replace(/^v/, "");
    if (!isNewer(remoteVersion, currentVersion)) return null;

    const platform = options.platform ?? detectPlatform();
    const asset = findAssetForPlatform(release.assets, platform);

    if (!asset) return null;

    return {
      version: release.tag_name,
      downloadUrl: asset.browser_download_url,
      notes: release.body ?? "",
      date: release.published_at,
    };
  } catch {
    return null;
  }
}

/** Replaces the current binary and restarts the app. */
export async function downloadAndInstall(
  update: UpdateInfo,
  onProgress?: (progress: DownloadProgress) => void,
): Promise<void> {
  const currentExe = process.execPath;
  const dir = path.dirname(currentExe);
  const ext = process.platform === "win32" ? ".exe" : "";
  const newPath = currentExe + ".new" + ext;
  const oldPath = currentExe + ".old" + ext;

  console.log(`[bunview] Downloading update ${update.version}...`);

  const res = await fetch(update.downloadUrl);
  if (!res.ok) throw new Error(`Download failed: HTTP ${res.status}`);

  const reader = res.body!.getReader();
  const contentLength = parseInt(res.headers.get("content-length") ?? "0");
  const chunks: Uint8Array[] = [];
  let downloaded = 0;

  while (true) {
    const { done, value } = await reader.read();
    if (done) break;
    chunks.push(value);
    downloaded += value.length;
    onProgress?.({
      downloaded,
      total: contentLength,
      percent: contentLength ? Math.round((downloaded / contentLength) * 100) : 0,
    });
  }

  const data = new Uint8Array(downloaded);
  let offset = 0;
  for (const chunk of chunks) {
    data.set(chunk, offset);
    offset += chunk.length;
  }

  writeFileSync(newPath, data);
  if (process.platform !== "win32") chmodSync(newPath, 0o755);

  console.log(`[bunview] Installing update...`);

  if (process.platform === "win32") {
    // Windows can't replace a running .exe → defer to a batch script that runs after exit.
    const batPath = path.join(dir, "_bunview_update.bat");
    const batContent = [
      "@echo off",
      "timeout /t 1 /nobreak >nul",
      `move /Y "${currentExe}" "${oldPath}"`,
      `move /Y "${newPath}" "${currentExe}"`,
      `del "${oldPath}"`,
      `start "" "${currentExe}"`,
      `del "%~f0"`,
    ].join("\r\n");
    writeFileSync(batPath, batContent);

    Bun.spawn(["cmd", "/c", batPath], { stdout: "ignore", stderr: "ignore" });
    process.exit(0);
  } else {
    try {
      renameSync(currentExe, oldPath);
    } catch (err) {
      throw new Error(
        `Failed to back up current binary "${currentExe}" to "${oldPath}": ${err instanceof Error ? err.message : err}`,
      );
    }

    try {
      renameSync(newPath, currentExe);
    } catch (err) {
      try { renameSync(oldPath, currentExe); } catch {}
      throw new Error(
        `Failed to install new binary to "${currentExe}": ${err instanceof Error ? err.message : err}`,
      );
    }

    try { unlinkSync(oldPath); } catch {}

    console.log(`[bunview] Restarting...`);
    Bun.spawn([currentExe, ...process.argv.slice(1)], {
      stdout: "inherit", stderr: "inherit", stdin: "inherit",
    });
    process.exit(0);
  }
}

/** Periodic check — returns a cleanup fn. */
export function startAutoCheck(
  options: UpdateCheckOptions,
  onUpdate: (info: UpdateInfo) => void,
  intervalMs = 3_600_000,
): () => void {
  const timer = setInterval(async () => {
    const update = await check(options);
    if (update) onUpdate(update);
  }, intervalMs);

  check(options).then((u) => {
    if (u) onUpdate(u);
  });

  return () => clearInterval(timer);
}

function findAssetForPlatform(
  assets: { name: string; browser_download_url: string }[],
  platform: string,
): { name: string; browser_download_url: string } | undefined {
  const patterns = PLATFORM_PATTERNS[platform];
  if (!patterns) return undefined;

  const nameLower = (a: { name: string }) => a.name.toLowerCase();

  for (const pattern of patterns) {
    const match = assets.find((a) => nameLower(a).includes(pattern));
    if (match) return match;
  }

  return undefined;
}

function isNewer(remote: string, current: string): boolean {
  const r = remote.split(".").map(Number);
  const c = current.split(".").map(Number);
  for (let i = 0; i < Math.max(r.length, c.length); ++i) {
    const rv = r[i] ?? 0;
    const cv = c[i] ?? 0;
    if (rv > cv) return true;
    if (rv < cv) return false;
  }
  return false;
}

function detectPlatform(): string {
  const arch = process.arch === "arm64" ? "arm64" : "x64";
  if (process.platform === "win32")  return `windows-${arch}`;
  if (process.platform === "darwin") return `macos-${arch}`;
  return `linux-${arch}`;
}
