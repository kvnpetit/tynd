import type { Target } from "../types";
import type { FileAssociation } from "../../../src/types";
import { generateWindowsPortable, generateWindowsInstaller } from "./windows-nsis";
import { generateWindowsMsi } from "./windows-msi";
import { generateMacAppBundle } from "./macos-app";
import { generateMacDmg } from "./macos-dmg";
import { generateAppImage } from "./linux-appimage";
import { generateDebPackage } from "./linux-deb";
import { generateRpmPackage } from "./linux-rpm";

type Icons = { ico?: string; icns?: string; png?: string };

/** Portable artifact: NSIS SFX (Windows) / .app (macOS) / AppImage (Linux). */
export async function generatePortable(
  exePath: string,
  appName: string,
  target: Target,
  outDir: string,
  icons: Icons,
  urlScheme?: string,
  fileAssociations?: FileAssociation[],
) {
  if (target.startsWith("windows")) {
    await generateWindowsPortable(exePath, appName, outDir, icons.ico);
  } else if (target.startsWith("macos")) {
    await generateMacAppBundle(exePath, appName, outDir, icons.icns, urlScheme, fileAssociations);
  } else {
    await generateAppImage(exePath, appName, outDir, icons.png, urlScheme, fileAssociations);
  }
}

/** Full installers: NSIS + MSI (Windows), .dmg (macOS), .deb + .rpm (Linux). */
export async function generateInstaller(
  exePath: string,
  appName: string,
  target: Target,
  outDir: string,
  icons: Icons,
  urlScheme?: string,
  fileAssociations?: FileAssociation[],
) {
  if (target.startsWith("windows")) {
    await generateWindowsInstaller(exePath, appName, outDir, icons.ico, urlScheme, fileAssociations);
    await generateWindowsMsi(exePath, appName, outDir, icons.ico, urlScheme, fileAssociations);
  } else if (target.startsWith("macos")) {
    await generateMacDmg(exePath, appName, outDir, icons.icns, urlScheme, fileAssociations);
  } else {
    await generateDebPackage(exePath, appName, outDir, icons.png, urlScheme, fileAssociations);
    await generateRpmPackage(exePath, appName, outDir, icons.png, urlScheme, fileAssociations);
  }
}
