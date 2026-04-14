import { fileURLToPath } from "url";
import { mkdirSync } from "fs";
import path from "path";
import type { FileAssociation } from "../../../src/types";

/** Distro-agnostic Linux portable. Auto-downloads `appimagetool` on first use. */
export async function generateAppImage(exePath: string, appName: string, outDir: string, pngPath?: string, urlScheme?: string, fileAssociations?: FileAssociation[]) {
  const { copyFileSync, writeFileSync, chmodSync } = await import("fs");
  const bunviewRoot = fileURLToPath(new URL("../../../", import.meta.url));
  const appimageDir = path.join(outDir, `${appName}.AppDir`);

  mkdirSync(path.join(appimageDir, "usr", "bin"), { recursive: true });
  copyFileSync(exePath, path.join(appimageDir, "usr", "bin", appName));
  chmodSync(path.join(appimageDir, "usr", "bin", appName), 0o755);

  if (pngPath) {
    copyFileSync(pngPath, path.join(appimageDir, `${appName.toLowerCase()}.png`));
  }

  writeFileSync(path.join(appimageDir, "AppRun"),
    `#!/bin/bash\nexec "$APPDIR/usr/bin/${appName}" "$@"\n`);
  chmodSync(path.join(appimageDir, "AppRun"), 0o755);

  const mimeParts = [
    ...(urlScheme ? [`x-scheme-handler/${urlScheme}`] : []),
    ...(fileAssociations?.flatMap((a) => a.mimeType ? [a.mimeType] : []) ?? []),
  ];
  writeFileSync(path.join(appimageDir, `${appName.toLowerCase()}.desktop`),
    `[Desktop Entry]\nName=${appName}\nExec=${appName} %U\nType=Application\nCategories=Utility;\n` +
    (pngPath ? `Icon=${appName.toLowerCase()}\n` : "") +
    (mimeParts.length ? `MimeType=${mimeParts.join(";")};\n` : ""));

  const appimgtool = path.join(bunviewRoot, "host", ".deps", "appimagetool");
  if (!await Bun.file(appimgtool).exists()) {
    console.log(`[bunview] Downloading appimagetool...`);
    const res = await fetch(
      "https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-x86_64.AppImage",
    );
    if (res.ok) {
      await Bun.write(appimgtool, await res.arrayBuffer());
      chmodSync(appimgtool, 0o755);
    }
  }

  if (await Bun.file(appimgtool).exists()) {
    const appimagePath = path.join(outDir, `${appName}.AppImage`);
    const proc = Bun.spawn([appimgtool, appimageDir, appimagePath], { stdout: "inherit", stderr: "inherit" });
    if (await proc.exited === 0) {
      const size = (Bun.file(appimagePath).size / 1024 / 1024).toFixed(1);
      console.log(`[bunview] ✅ AppImage: ${path.relative(process.cwd(), appimagePath)} (${size} MB)`);
    }
  }

  try { (await import("fs/promises")).rm(appimageDir, { recursive: true }); } catch {}
}
