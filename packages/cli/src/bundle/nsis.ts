import { copyFileSync, existsSync, mkdirSync, rmSync } from "node:fs"
import path from "node:path"
import { exec } from "../lib/exec.ts"
import { pngToIco } from "../lib/icon.ts"
import { log } from "../lib/logger.ts"
import { ICO_SIZES, renderIconPngSet } from "./icon-gen.ts"
import { signWindows } from "./sign.ts"
import { ensureTool, type ToolSpec } from "./tools.ts"
import type { BundleContext } from "./types.ts"

const NSIS: ToolSpec = {
  name: "nsis",
  version: "3.09",
  archive: "zip",
  binRel: "nsis-3.09/makensis.exe",
  url: (platform) =>
    platform === "windows"
      ? "https://downloads.sourceforge.net/project/nsis/NSIS%203/3.09/nsis-3.09.zip"
      : null,
}

export async function bundleNsis(ctx: BundleContext): Promise<string> {
  if (ctx.platform !== "windows") {
    throw new Error("NSIS installers can only be built on a Windows host")
  }

  const tool = await ensureTool(NSIS, ctx.toolsDir, ctx.platform, ctx.arch)

  const workDir = path.join(ctx.outDir, `.${ctx.appName}-nsis`)
  if (existsSync(workDir)) rmSync(workDir, { recursive: true, force: true })
  mkdirSync(workDir, { recursive: true })

  const exeName = `${ctx.appName}.exe`
  copyFileSync(ctx.inputBinary, path.join(workDir, exeName))

  let iconRel: string | null = null
  if (ctx.iconSource) {
    iconRel = `${ctx.appName}.ico`
    const dest = path.join(workDir, iconRel)
    if (path.extname(ctx.iconSource).toLowerCase() === ".ico") {
      copyFileSync(ctx.iconSource, dest)
    } else {
      const entries = await renderIconPngSet(ctx.iconSource, ICO_SIZES)
      await Bun.write(dest, pngToIco(entries))
    }
  }

  const outFile = path.join(ctx.outDir, `${ctx.appName}-${ctx.version}-setup.exe`)
  if (existsSync(outFile)) rmSync(outFile, { force: true })

  const scriptPath = path.join(workDir, `${ctx.appName}.nsi`)
  await Bun.write(scriptPath, renderScript(ctx, exeName, iconRel, outFile))

  log.step("Running makensis…")
  await exec(tool.bin, [scriptPath], { silent: true })

  if (!existsSync(outFile)) {
    throw new Error(`makensis finished without producing ${outFile}`)
  }

  // Sign the installer itself — otherwise users see a SmartScreen "Unknown
  // Publisher" warning even if the inner .exe is signed.
  await signWindows(ctx.bundleConfig, outFile)

  rmSync(workDir, { recursive: true, force: true })
  log.success(`NSIS     -> ${log.cyan(`release/${path.basename(outFile)}`)}`)
  return outFile
}

function renderScript(
  ctx: BundleContext,
  exeName: string,
  iconRel: string | null,
  outFile: string,
): string {
  const mode = ctx.bundleConfig.nsis?.installMode ?? "currentUser"
  const isUser = mode === "currentUser"
  const installRoot = isUser ? "$LOCALAPPDATA\\Programs" : "$PROGRAMFILES64"
  const execLevel = isUser ? "user" : "admin"
  const uninstallHive = isUser ? "HKCU" : "HKLM"
  const schemeHive = isUser ? "HKCU" : "HKLM"
  const uninstallKey = `Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\${ctx.identifier}`
  const publisher = ctx.author?.name ?? ""
  const iconDirective = iconRel ? `Icon "${iconRel}"\n!define MUI_ICON "${iconRel}"` : ""

  // NSIS string escaping:
  //   `$$`   for a literal `$` (otherwise NSIS expands $VARIABLE references)
  //   `\\`   for backslash
  //   `$\"` for an embedded double-quote
  // Order matters: escape `$` first so the other escapes don't themselves
  // introduce `$` sequences we then re-rewrite.
  const nsisStr = (s: string) =>
    s.replace(/\$/g, "$$$$").replace(/\\/g, "\\\\").replace(/"/g, '$\\"')

  const protocolInstall = ctx.protocols
    .map((scheme) => {
      const base = `Software\\Classes\\${scheme}`
      const cmd = `"$INSTDIR\\${exeName}" "%1"`
      return `    WriteRegStr ${schemeHive} "${base}" "" "URL:${nsisStr(ctx.displayName)}"
    WriteRegStr ${schemeHive} "${base}" "URL Protocol" ""
    WriteRegStr ${schemeHive} "${base}\\DefaultIcon" "" "$INSTDIR\\${exeName},0"
    WriteRegStr ${schemeHive} "${base}\\shell\\open\\command" "" \`${cmd}\``
    })
    .join("\n")
  const protocolUninstall = ctx.protocols
    .map((scheme) => `    DeleteRegKey ${schemeHive} "Software\\Classes\\${scheme}"`)
    .join("\n")

  return `!include "MUI2.nsh"

Name "${nsisStr(ctx.displayName)}"
OutFile "${nsisStr(outFile)}"
Unicode true
InstallDir "${installRoot}\\${nsisStr(ctx.displayName)}"
InstallDirRegKey ${uninstallHive} "${uninstallKey}" "InstallLocation"
RequestExecutionLevel ${execLevel}
${iconDirective}

VIProductVersion "${ctx.version}.0"
VIAddVersionKey "ProductName" "${nsisStr(ctx.displayName)}"
VIAddVersionKey "CompanyName" "${nsisStr(publisher)}"
VIAddVersionKey "FileDescription" "${nsisStr(ctx.shortDescription)}"
VIAddVersionKey "FileVersion" "${ctx.version}"
VIAddVersionKey "LegalCopyright" "${nsisStr(ctx.copyright)}"

!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"

Section "Install"
    SetOutPath "$INSTDIR"
    File "${exeName}"
${iconRel ? `    File "${iconRel}"` : ""}
    WriteUninstaller "$INSTDIR\\uninstall.exe"

    CreateShortCut "$DESKTOP\\${nsisStr(ctx.displayName)}.lnk" "$INSTDIR\\${exeName}"
    CreateDirectory "$SMPROGRAMS\\${nsisStr(ctx.displayName)}"
    CreateShortCut "$SMPROGRAMS\\${nsisStr(ctx.displayName)}\\${nsisStr(ctx.displayName)}.lnk" "$INSTDIR\\${exeName}"

    WriteRegStr ${uninstallHive} "${uninstallKey}" "DisplayName" "${nsisStr(ctx.displayName)}"
    WriteRegStr ${uninstallHive} "${uninstallKey}" "UninstallString" "$INSTDIR\\uninstall.exe"
    WriteRegStr ${uninstallHive} "${uninstallKey}" "InstallLocation" "$INSTDIR"
    WriteRegStr ${uninstallHive} "${uninstallKey}" "DisplayVersion" "${ctx.version}"
    WriteRegStr ${uninstallHive} "${uninstallKey}" "Publisher" "${nsisStr(publisher)}"
${iconRel ? `    WriteRegStr ${uninstallHive} "${uninstallKey}" "DisplayIcon" "$INSTDIR\\${iconRel}"` : ""}
    WriteRegDWORD ${uninstallHive} "${uninstallKey}" "NoModify" 1
    WriteRegDWORD ${uninstallHive} "${uninstallKey}" "NoRepair" 1
${protocolInstall}
SectionEnd

Section "Uninstall"
    Delete "$INSTDIR\\${exeName}"
${iconRel ? `    Delete "$INSTDIR\\${iconRel}"` : ""}
    Delete "$INSTDIR\\uninstall.exe"
    Delete "$DESKTOP\\${nsisStr(ctx.displayName)}.lnk"
    Delete "$SMPROGRAMS\\${nsisStr(ctx.displayName)}\\${nsisStr(ctx.displayName)}.lnk"
    RMDir "$SMPROGRAMS\\${nsisStr(ctx.displayName)}"
    RMDir "$INSTDIR"
    DeleteRegKey ${uninstallHive} "${uninstallKey}"
${protocolUninstall}
SectionEnd
`
}
