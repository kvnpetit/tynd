import type { FileAssociation } from "../../../src/types";
import { generateLinuxPackage } from "./linux-nfpm";

/** RPM package (.rpm) — built via auto-downloaded nfpm, no `rpmbuild` needed. */
export function generateRpmPackage(exePath: string, appName: string, outDir: string, pngPath?: string, urlScheme?: string, fileAssociations?: FileAssociation[]) {
  return generateLinuxPackage("rpm", exePath, appName, outDir, pngPath, urlScheme, fileAssociations);
}
