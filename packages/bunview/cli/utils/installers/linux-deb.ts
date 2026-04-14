import type { FileAssociation } from "../../../src/types";
import { generateLinuxPackage } from "./linux-nfpm";

/** Debian package (.deb) — built via auto-downloaded nfpm, no `dpkg-deb` needed. */
export function generateDebPackage(exePath: string, appName: string, outDir: string, pngPath?: string, urlScheme?: string, fileAssociations?: FileAssociation[]) {
  return generateLinuxPackage("deb", exePath, appName, outDir, pngPath, urlScheme, fileAssociations);
}
