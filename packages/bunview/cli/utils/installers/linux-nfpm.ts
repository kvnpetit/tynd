import { fileURLToPath } from "url";
import { mkdirSync, writeFileSync, copyFileSync, chmodSync } from "fs";
import path from "path";
import os from "os";
import type { FileAssociation } from "../../../src/types";
import { ensureNfpm } from "./shared";

type Packager = "deb" | "rpm";

/** Build `.deb` or `.rpm` via auto-downloaded nfpm — no `dpkg-deb`/`rpmbuild` needed. */
export async function generateLinuxPackage(
  packager: Packager,
  exePath: string,
  appName: string,
  outDir: string,
  pngPath?: string,
  urlScheme?: string,
  fileAssociations?: FileAssociation[],
): Promise<void> {
  const bunviewRoot = fileURLToPath(new URL("../../../", import.meta.url));
  const nfpm = await ensureNfpm(bunviewRoot);
  if (!nfpm) {
    console.log(`[bunview] ⚠  Skipped .${packager} — nfpm unavailable.`);
    return;
  }

  // nfpm copies files by path — materialize the .desktop in a staging dir.
  const stagingDir  = path.join(os.tmpdir(), `bunview-nfpm-${appName}-${Date.now()}`);
  const desktopPath = path.join(stagingDir, `${appName.toLowerCase()}.desktop`);
  mkdirSync(stagingDir, { recursive: true });

  const mimeParts = [
    ...(urlScheme ? [`x-scheme-handler/${urlScheme}`] : []),
    ...(fileAssociations?.flatMap((a) => a.mimeType ? [a.mimeType] : []) ?? []),
  ];
  writeFileSync(desktopPath,
    `[Desktop Entry]\nName=${appName}\nExec=/usr/local/bin/${appName} %U\nType=Application\nCategories=Utility;\n` +
    (pngPath ? `Icon=${appName.toLowerCase()}\n` : "") +
    (mimeParts.length ? `MimeType=${mimeParts.join(";")};\n` : ""));

  const contents: Array<{ src: string; dst: string; type?: string }> = [
    { src: exePath,     dst: `/usr/local/bin/${appName}` },
    { src: desktopPath, dst: `/usr/share/applications/${appName.toLowerCase()}.desktop` },
  ];
  if (pngPath) {
    const stagingPng = path.join(stagingDir, `${appName.toLowerCase()}.png`);
    copyFileSync(pngPath, stagingPng);
    contents.push({ src: stagingPng, dst: `/usr/share/icons/hicolor/256x256/apps/${appName.toLowerCase()}.png` });
  }

  const yaml =
`name: ${appName.toLowerCase()}
arch: amd64
platform: linux
version: 1.0.0
section: utils
priority: optional
maintainer: bunview
description: |
  ${appName} desktop application built with bunview.
license: Proprietary
contents:
${contents.map(c => `  - src: ${JSON.stringify(c.src)}\n    dst: ${JSON.stringify(c.dst)}${c.type ? `\n    type: ${c.type}` : ""}`).join("\n")}
`;
  const yamlPath = path.join(stagingDir, "nfpm.yaml");
  writeFileSync(yamlPath, yaml);

  const ext    = packager === "deb" ? "deb" : "rpm";
  const target = path.join(outDir, `${appName}.${ext}`);
  const proc   = Bun.spawn(
    [nfpm, "pkg", "--packager", packager, "--config", yamlPath, "--target", target],
    { stdout: "pipe", stderr: "pipe" },
  );
  const code = await proc.exited;

  if (code !== 0) {
    console.error(`[bunview] ❌ nfpm (${packager}) failed:\n${await new Response(proc.stderr).text()}`);
  } else if (await Bun.file(target).exists()) {
    // nfpm copies the mode bits we set on the source.
    try { chmodSync(exePath, 0o755); } catch {}
    const size = (Bun.file(target).size / 1024 / 1024).toFixed(1);
    console.log(`[bunview] ✅ ${ext.toUpperCase()}: ${path.relative(process.cwd(), target)} (${size} MB)`);
  }

  try { (await import("fs/promises")).rm(stagingDir, { recursive: true }); } catch {}
}
