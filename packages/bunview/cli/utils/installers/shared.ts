import { mkdirSync } from "fs";
import path from "path";

async function extractZip(zipPath: string, destDir: string): Promise<boolean> {
  const cmd = process.platform === "win32"
    ? ["powershell", "-command", `Expand-Archive -Path '${zipPath}' -DestinationPath '${destDir}' -Force`]
    : ["unzip", "-oq", zipPath, "-d", destDir];
  const code = await Bun.spawn(cmd, { stdout: "pipe", stderr: "pipe" }).exited;
  return code === 0;
}

async function extractTarGz(archive: string, destDir: string): Promise<boolean> {
  const code = await Bun.spawn(["tar", "-xzf", archive, "-C", destDir], { stdout: "pipe", stderr: "pipe" }).exited;
  return code === 0;
}

export async function fetchAndExtract(url: string, destDir: string, archiveName: string): Promise<boolean> {
  mkdirSync(destDir, { recursive: true });
  const archivePath = path.join(destDir, archiveName);
  const res = await fetch(url);
  if (!res.ok) return false;
  await Bun.write(archivePath, await res.arrayBuffer());
  const ok = archiveName.endsWith(".zip") ? await extractZip(archivePath, destDir) : await extractTarGz(archivePath, destDir);
  try { (await import("fs")).unlinkSync(archivePath); } catch {}
  return ok;
}

export async function which(cmd: string): Promise<string | null> {
  const probe = process.platform === "win32" ? ["where", cmd] : ["which", cmd];
  const proc  = Bun.spawn(probe, { stdout: "pipe", stderr: "pipe" });
  const code  = await proc.exited;
  if (code !== 0) return null;
  const out = await new Response(proc.stdout).text();
  return out.split(/\r?\n/)[0]?.trim() || null;
}

export function xmlEscape(s: string): string {
  return s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;").replace(/'/g, "&apos;");
}

/** UUID v4-shaped GUID, stable per seed (via SHA-256). */
export function deterministicGuid(seed: string): string {
  const hasher = new Bun.CryptoHasher("sha256");
  hasher.update(seed);
  const h = hasher.digest("hex");
  return `${h.slice(0, 8)}-${h.slice(8, 12)}-4${h.slice(13, 16)}-a${h.slice(17, 20)}-${h.slice(20, 32)}`.toUpperCase();
}

export async function ensureNSIS(bunviewRoot: string): Promise<string | null> {
  const depsDir  = path.join(bunviewRoot, "host", ".deps");
  const nsisDir  = path.join(depsDir, "nsis");
  const makensis = path.join(nsisDir, "makensis.exe");

  if (await Bun.file(makensis).exists()) return makensis;

  console.log(`[bunview] Downloading NSIS portable...`);
  mkdirSync(nsisDir, { recursive: true });

  const zipPath = path.join(depsDir, "nsis.zip");
  const urls = [
    "https://github.com/nicehash/NSIS/releases/download/v3.09/nsis-3.09-portable.zip",
    "https://sourceforge.net/projects/nsis/files/NSIS%203/3.09/nsis-3.09.zip/download",
  ];

  let downloaded = false;
  for (const url of urls) {
    const res = await fetch(url);
    if (res.ok) {
      await Bun.write(zipPath, await res.arrayBuffer());
      downloaded = true;
      break;
    }
  }

  if (!downloaded) {
    console.error(`[bunview] ❌ Cannot download NSIS.`);
    return null;
  }

  await Bun.spawn(
    ["powershell", "-command", `Expand-Archive -Path '${zipPath}' -DestinationPath '${nsisDir}' -Force`],
    { stdout: "inherit", stderr: "inherit" },
  ).exited;

  // Flatten a single nested directory if the archive wrapped everything in one.
  const { readdirSync, renameSync } = await import("fs");
  for (const sub of readdirSync(nsisDir)) {
    const candidate = path.join(nsisDir, sub, "makensis.exe");
    if (await Bun.file(candidate).exists()) {
      const subDir = path.join(nsisDir, sub);
      for (const f of readdirSync(subDir)) {
        try { renameSync(path.join(subDir, f), path.join(nsisDir, f)); } catch {}
      }
      break;
    }
  }

  if (!await Bun.file(makensis).exists()) {
    console.error(`[bunview] ❌ NSIS downloaded but makensis.exe not found.`);
    return null;
  }

  console.log(`[bunview] NSIS ready.`);
  return makensis;
}

/** WiX v3 — standalone zip, no .NET required unlike v4. */
export async function ensureWix(bunviewRoot: string): Promise<{ candle: string; light: string } | null> {
  const depsDir = path.join(bunviewRoot, "host", ".deps");
  const wixDir  = path.join(depsDir, "wix");
  const candle  = path.join(wixDir, "candle.exe");
  const light   = path.join(wixDir, "light.exe");

  if (await Bun.file(candle).exists() && await Bun.file(light).exists()) {
    return { candle, light };
  }

  console.log(`[bunview] Downloading WiX v3 portable...`);
  const url = "https://github.com/wixtoolset/wix3/releases/download/wix3141rtm/wix314-binaries.zip";
  if (!await fetchAndExtract(url, wixDir, "wix.zip")) {
    console.error(`[bunview] ❌ Cannot download WiX.`);
    return null;
  }
  if (!await Bun.file(candle).exists() || !await Bun.file(light).exists()) {
    console.error(`[bunview] ❌ WiX downloaded but candle/light not found.`);
    return null;
  }
  console.log(`[bunview] WiX ready.`);
  return { candle, light };
}

/** Static Go binary that builds `.deb`/`.rpm`/`.apk`/Arch from one YAML config. */
export async function ensureNfpm(bunviewRoot: string): Promise<string | null> {
  const depsDir = path.join(bunviewRoot, "host", ".deps");
  const nfpmDir = path.join(depsDir, "nfpm");
  const exe     = process.platform === "win32" ? "nfpm.exe" : "nfpm";
  const nfpmBin = path.join(nfpmDir, exe);

  if (await Bun.file(nfpmBin).exists()) return nfpmBin;

  const version = "2.41.3";
  const os   = process.platform === "win32" ? "Windows" : process.platform === "darwin" ? "Darwin" : "Linux";
  const arch = process.arch === "arm64" ? "arm64" : "x86_64";
  const ext  = process.platform === "win32" ? "zip" : "tar.gz";
  const url  = `https://github.com/goreleaser/nfpm/releases/download/v${version}/nfpm_${version}_${os}_${arch}.${ext}`;

  console.log(`[bunview] Downloading nfpm v${version}...`);
  if (!await fetchAndExtract(url, nfpmDir, `nfpm.${ext}`)) {
    console.error(`[bunview] ❌ Cannot download nfpm.`);
    return null;
  }
  if (!await Bun.file(nfpmBin).exists()) {
    console.error(`[bunview] ❌ nfpm downloaded but binary not found.`);
    return null;
  }
  if (process.platform !== "win32") {
    try { (await import("fs")).chmodSync(nfpmBin, 0o755); } catch {}
  }
  console.log(`[bunview] nfpm ready.`);
  return nfpmBin;
}

/** Set `BUNVIEW_DEBUG=1` for full NSIS output. */
export async function runNsis(makensis: string, nsiPath: string): Promise<number> {
  if (process.env.BUNVIEW_DEBUG) {
    return (await Bun.spawn([makensis, nsiPath], { stdout: "inherit", stderr: "inherit" }).exited);
  }
  const proc = Bun.spawn([makensis, nsiPath], { stdout: "pipe", stderr: "pipe" });
  const out  = await new Response(proc.stdout).text();
  const code = await proc.exited;

  if (code !== 0) {
    console.error(out);
    console.error(await new Response(proc.stderr).text());
  } else {
    const outLine = out.split("\n").find((l) => /^Output: /.test(l));
    if (outLine) console.log(`  ${outLine.trim()}`);
  }
  return code;
}
