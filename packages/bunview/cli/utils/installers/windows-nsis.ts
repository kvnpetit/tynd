import { fileURLToPath } from "url";
import path from "path";
import type { FileAssociation } from "../../../src/types";
import { ensureNSIS, runNsis } from "./shared";

/** Silent SFX — extracts to %LOCALAPPDATA%\AppName and launches. */
export async function generateWindowsPortable(exePath: string, appName: string, outDir: string, icoPath?: string) {
  const bunviewRoot = fileURLToPath(new URL("../../../", import.meta.url));
  const makensis    = await ensureNSIS(bunviewRoot);
  if (!makensis) return;

  const portablePath = path.join(outDir, `${appName}-portable.exe`);
  const nsiPath      = path.join(outDir, `${appName}-portable.nsi`);

  const nsiScript = `
!include "MUI2.nsh"

Name "${appName}"
OutFile "${portablePath.replace(/\\/g, "\\\\")}"
InstallDir "$LOCALAPPDATA\\\\${appName}"
RequestExecutionLevel user
SilentInstall silent
AutoCloseWindow true

${icoPath ? `!define MUI_ICON "${icoPath.replace(/\\/g, "\\\\")}"` : ""}

Section
  SetOutPath $INSTDIR
  File "${exePath.replace(/\\/g, "\\\\")}"
  ExecShell "open" '"$INSTDIR\\\\${path.basename(exePath)}"'
SectionEnd
`;

  await Bun.write(nsiPath, nsiScript);
  const code = await runNsis(makensis, nsiPath);
  try { (await import("fs")).unlinkSync(nsiPath); } catch {}

  if (code === 0 && await Bun.file(portablePath).exists()) {
    const size = (Bun.file(portablePath).size / 1024 / 1024).toFixed(1);
    console.log(`  → ${path.relative(process.cwd(), portablePath)} (${size} MB)`);
  } else {
    console.error(`[bunview] ❌ Portable SFX generation failed.`);
  }
}

/** Full NSIS wizard with shortcuts, uninstaller, URL scheme + file assoc registry entries. */
export async function generateWindowsInstaller(exePath: string, appName: string, outDir: string, icoPath?: string, urlScheme?: string, fileAssociations?: FileAssociation[]) {
  const bunviewRoot = fileURLToPath(new URL("../../../", import.meta.url));
  const makensis    = await ensureNSIS(bunviewRoot);
  if (!makensis) return;

  const installerPath = path.join(outDir, `${appName}-setup.exe`);
  const nsiPath       = path.join(outDir, `${appName}-setup.nsi`);

  const nsiScript = `
!include "MUI2.nsh"

Name "${appName}"
OutFile "${installerPath.replace(/\\/g, "\\\\")}"
InstallDir "$LOCALAPPDATA\\\\${appName}"
RequestExecutionLevel user

${icoPath ? `!define MUI_ICON "${icoPath.replace(/\\/g, "\\\\")}"` : ""}
${icoPath ? `!define MUI_UNICON "${icoPath.replace(/\\/g, "\\\\")}"` : ""}

!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"

Section "Install"
  SetOutPath $INSTDIR
  File "${exePath.replace(/\\/g, "\\\\")}"
  CreateShortcut "$DESKTOP\\\\${appName}.lnk" "$INSTDIR\\\\${path.basename(exePath)}"
  CreateShortcut "$SMPROGRAMS\\\\${appName}.lnk" "$INSTDIR\\\\${path.basename(exePath)}"
  WriteUninstaller "$INSTDIR\\\\uninstall.exe"
  WriteRegStr HKCU "Software\\\\Microsoft\\\\Windows\\\\CurrentVersion\\\\Uninstall\\\\${appName}" "DisplayName" "${appName}"
  WriteRegStr HKCU "Software\\\\Microsoft\\\\Windows\\\\CurrentVersion\\\\Uninstall\\\\${appName}" "UninstallString" "$INSTDIR\\\\uninstall.exe"
  ${icoPath ? `WriteRegStr HKCU "Software\\\\Microsoft\\\\Windows\\\\CurrentVersion\\\\Uninstall\\\\${appName}" "DisplayIcon" "$INSTDIR\\\\${path.basename(exePath)}"` : ""}
${urlScheme ? `  WriteRegStr HKCU "Software\\\\Classes\\\\${urlScheme}" "" "URL:${appName} Protocol"
  WriteRegStr HKCU "Software\\\\Classes\\\\${urlScheme}" "URL Protocol" ""
  WriteRegStr HKCU "Software\\\\Classes\\\\${urlScheme}\\\\DefaultIcon" "" "$INSTDIR\\\\${path.basename(exePath)},0"
  WriteRegStr HKCU "Software\\\\Classes\\\\${urlScheme}\\\\shell\\\\open\\\\command" "" '"$INSTDIR\\\\${path.basename(exePath)}" "%1"'` : ""}
${(fileAssociations ?? []).map(a => `  WriteRegStr HKCU "Software\\\\Classes\\\\.${a.ext}" "" "${appName}.${a.ext}"
  WriteRegStr HKCU "Software\\\\Classes\\\\${appName}.${a.ext}" "" "${a.name}"
  WriteRegStr HKCU "Software\\\\Classes\\\\${appName}.${a.ext}\\\\DefaultIcon" "" "$INSTDIR\\\\${path.basename(exePath)},0"
  WriteRegStr HKCU "Software\\\\Classes\\\\${appName}.${a.ext}\\\\shell\\\\open\\\\command" "" '"$INSTDIR\\\\${path.basename(exePath)}" "%1"'`).join("\n")}
SectionEnd

Section "Uninstall"
  Delete "$INSTDIR\\\\${path.basename(exePath)}"
  Delete "$INSTDIR\\\\uninstall.exe"
  Delete "$DESKTOP\\\\${appName}.lnk"
  Delete "$SMPROGRAMS\\\\${appName}.lnk"
  RMDir "$INSTDIR"
  DeleteRegKey HKCU "Software\\\\Microsoft\\\\Windows\\\\CurrentVersion\\\\Uninstall\\\\${appName}"
${urlScheme ? `  DeleteRegKey HKCU "Software\\\\Classes\\\\${urlScheme}"` : ""}
${(fileAssociations ?? []).map(a => `  DeleteRegKey HKCU "Software\\\\Classes\\\\.${a.ext}"
  DeleteRegKey HKCU "Software\\\\Classes\\\\${appName}.${a.ext}"`).join("\n")}
SectionEnd
`;

  await Bun.write(nsiPath, nsiScript);
  const code = await runNsis(makensis, nsiPath);
  try { (await import("fs")).unlinkSync(nsiPath); } catch {}

  if (code === 0 && await Bun.file(installerPath).exists()) {
    const size = (Bun.file(installerPath).size / 1024 / 1024).toFixed(1);
    console.log(`  → ${path.relative(process.cwd(), installerPath)} (${size} MB)`);
  } else {
    console.error(`[bunview] ❌ Installer generation failed.`);
  }
}
