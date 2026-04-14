import path from "path";
import type { FileAssociation } from "../../../src/types";
import { generateMacAppBundle } from "./macos-app";

/** `.dmg` wrapping the `.app` bundle via `hdiutil`. Rebuilds the bundle as payload. */
export async function generateMacDmg(exePath: string, appName: string, outDir: string, icnsPath?: string, urlScheme?: string, fileAssociations?: FileAssociation[]) {
  await generateMacAppBundle(exePath, appName, outDir, icnsPath, urlScheme, fileAssociations);

  const appDir  = path.join(outDir, `${appName}.app`);
  const dmgPath = path.join(outDir, `${appName}.dmg`);
  const proc    = Bun.spawn(
    ["hdiutil", "create", "-volname", appName, "-srcfolder", appDir, "-ov", dmgPath],
    { stdout: "inherit", stderr: "inherit" },
  );
  if (await proc.exited === 0) {
    const size = (Bun.file(dmgPath).size / 1024 / 1024).toFixed(1);
    console.log(`[bunview] ✅ DMG: ${path.relative(process.cwd(), dmgPath)} (${size} MB)`);
  }
}
