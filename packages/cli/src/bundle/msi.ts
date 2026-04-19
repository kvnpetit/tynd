import { copyFileSync, existsSync, mkdirSync, rmSync } from "node:fs"
import path from "node:path"
import { exec } from "../lib/exec.ts"
import { pngToIco } from "../lib/icon.ts"
import { log } from "../lib/logger.ts"
import { ICO_SIZES, renderIconPngSet } from "./icon-gen.ts"
import { signWindows } from "./sign.ts"
import { ensureTool, type ToolSpec } from "./tools.ts"
import type { BundleContext } from "./types.ts"

// WiX v3 (not v4) — flat zip, no .NET SDK required.
const WIX3: ToolSpec = {
  name: "wix",
  version: "3.11.2",
  archive: "zip",
  binRel: "candle.exe",
  url: (platform) =>
    platform === "windows"
      ? "https://github.com/wixtoolset/wix3/releases/download/wix3112rtm/wix311-binaries.zip"
      : null,
}

export async function bundleMsi(ctx: BundleContext): Promise<string> {
  if (ctx.platform !== "windows") {
    throw new Error("MSI installers can only be built on a Windows host")
  }

  const tool = await ensureTool(WIX3, ctx.toolsDir, ctx.platform, ctx.arch)
  const candleExe = tool.bin
  const lightExe = path.join(tool.dir, "light.exe")

  const workDir = path.join(ctx.outDir, `.${ctx.appName}-msi`)
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

  const msiVersion = toMsiVersion(ctx.version)
  const upgradeCode =
    ctx.bundleConfig.msi?.upgradeCode ?? deterministicGuid(`upgrade:${ctx.identifier}`)

  const wxsPath = path.join(workDir, `${ctx.appName}.wxs`)
  await Bun.write(wxsPath, renderWxs(ctx, exeName, iconRel, msiVersion, upgradeCode))

  const wixObj = path.join(workDir, `${ctx.appName}.wixobj`)
  const outFile = path.join(ctx.outDir, `${ctx.appName}-${ctx.version}-${ctx.arch}.msi`)
  if (existsSync(outFile)) rmSync(outFile, { force: true })

  const candleArch = ctx.arch === "arm64" ? "arm64" : "x64"

  log.step("Running candle (WiX compile)…")
  await exec(candleExe, ["-arch", candleArch, "-out", wixObj, wxsPath], { silent: true })

  log.step("Running light (WiX link)…")
  await exec(lightExe, ["-ext", "WixUIExtension", "-out", outFile, wixObj], { silent: true })

  if (!existsSync(outFile)) {
    throw new Error(`light finished without producing ${outFile}`)
  }

  await signWindows(ctx.bundleConfig, outFile)

  rmSync(workDir, { recursive: true, force: true })
  log.success(`MSI      -> ${log.cyan(`release/${path.basename(outFile)}`)}`)
  return outFile
}

// MSI versions are 4-part major.minor.build.revision; trim semver prerelease.
function toMsiVersion(semver: string): string {
  const m = semver.match(/^(\d+)\.(\d+)\.(\d+)/)
  if (!m) return "0.0.0.0"
  return `${m[1]}.${m[2]}.${m[3]}.0`
}

// Stable GUID from a string — used for UpgradeCode when the user doesn't pin one.
function deterministicGuid(input: string): string {
  const hasher = new Bun.CryptoHasher("sha1")
  hasher.update(`tynd:${input}`)
  const h = hasher.digest("hex")
  return `${h.slice(0, 8)}-${h.slice(8, 12)}-5${h.slice(12, 15)}-a${h.slice(15, 18)}-${h.slice(18, 30)}`.toUpperCase()
}

function renderWxs(
  ctx: BundleContext,
  exeName: string,
  iconRel: string | null,
  msiVersion: string,
  upgradeCode: string,
): string {
  const manufacturer = esc(ctx.author?.name ?? "Unknown")
  const displayName = esc(ctx.displayName)
  const iconBlock = iconRel
    ? `<Icon Id="AppIcon.ico" SourceFile="${esc(iconRel)}" />\n      <Property Id="ARPPRODUCTICON" Value="AppIcon.ico" />`
    : ""
  const shortcutIconAttr = iconRel ? `Icon="AppIcon.ico"` : ""

  // Per-scheme registry components — installed to HKCU so we don't need admin
  // rights. Matches NSIS currentUser default.
  const schemeComponents = ctx.protocols
    .map((scheme, i) => {
      const compId = `Scheme_${i}`
      const base = `Software\\Classes\\${scheme}`
      return `      <Component Id="${compId}" Guid="*">
        <RegistryKey Root="HKCU" Key="${base}">
          <RegistryValue Type="string" Value="URL:${esc(ctx.displayName)}" KeyPath="yes"/>
          <RegistryValue Name="URL Protocol" Type="string" Value=""/>
        </RegistryKey>
        <RegistryKey Root="HKCU" Key="${base}\\DefaultIcon">
          <RegistryValue Type="string" Value="[APPLICATIONFOLDER]${esc(exeName)},0"/>
        </RegistryKey>
        <RegistryKey Root="HKCU" Key="${base}\\shell\\open\\command">
          <RegistryValue Type="string" Value="&quot;[APPLICATIONFOLDER]${esc(exeName)}&quot; &quot;%1&quot;"/>
        </RegistryKey>
      </Component>`
    })
    .join("\n")
  const schemeRefs = ctx.protocols
    .map((_, i) => `      <ComponentRef Id="Scheme_${i}"/>`)
    .join("\n")

  return `<?xml version="1.0" encoding="UTF-8"?>
<Wix xmlns="http://schemas.microsoft.com/wix/2006/wi">
  <Product Id="*"
           Name="${displayName}"
           Language="1033"
           Version="${msiVersion}"
           Manufacturer="${manufacturer}"
           UpgradeCode="${upgradeCode}">
    <Package InstallerVersion="200" Compressed="yes" InstallScope="perUser" />
    <MajorUpgrade DowngradeErrorMessage="A newer version of ${displayName} is already installed." />
    <MediaTemplate EmbedCab="yes" />
    <Property Id="ALLUSERS" Value="" />
    <Property Id="MSIINSTALLPERUSER" Value="1" />

    ${iconBlock}

    <Feature Id="ProductFeature" Title="${displayName}" Level="1">
      <ComponentGroupRef Id="ProductComponents" />
      <ComponentRef Id="ApplicationShortcuts" />
${schemeRefs}
    </Feature>

    <Directory Id="TARGETDIR" Name="SourceDir">
      <Directory Id="LocalAppDataFolder">
        <Directory Id="ProgramsFolder" Name="Programs">
          <Directory Id="APPLICATIONFOLDER" Name="${displayName}" />
        </Directory>
      </Directory>
      <Directory Id="ProgramMenuFolder">
        <Directory Id="ApplicationProgramsFolder" Name="${displayName}" />
      </Directory>
      <Directory Id="DesktopFolder" Name="Desktop" />
    </Directory>

    <ComponentGroup Id="ProductComponents" Directory="APPLICATIONFOLDER">
      <Component Id="MainExecutable" Guid="*">
        <File Id="MainExe" Name="${esc(exeName)}" Source="${esc(exeName)}" KeyPath="yes" />
      </Component>
${
  iconRel
    ? `      <Component Id="AppIconComponent" Guid="*">
        <File Id="AppIconFile" Name="${esc(iconRel)}" Source="${esc(iconRel)}" KeyPath="yes" />
      </Component>`
    : ""
}
${schemeComponents}
    </ComponentGroup>

    <DirectoryRef Id="ApplicationProgramsFolder">
      <Component Id="ApplicationShortcuts" Guid="*">
        <Shortcut Id="StartMenuShortcut"
                  Name="${displayName}"
                  Description="${esc(ctx.shortDescription)}"
                  Target="[APPLICATIONFOLDER]${esc(exeName)}"
                  WorkingDirectory="APPLICATIONFOLDER"
                  ${shortcutIconAttr} />
        <Shortcut Id="DesktopShortcut"
                  Directory="DesktopFolder"
                  Name="${displayName}"
                  Description="${esc(ctx.shortDescription)}"
                  Target="[APPLICATIONFOLDER]${esc(exeName)}"
                  WorkingDirectory="APPLICATIONFOLDER"
                  ${shortcutIconAttr} />
        <RemoveFolder Id="CleanupShortcutFolder" On="uninstall" />
        <RegistryValue Root="HKCU"
                       Key="Software\\${esc(ctx.identifier)}"
                       Name="installed"
                       Type="integer"
                       Value="1"
                       KeyPath="yes" />
      </Component>
    </DirectoryRef>
  </Product>
</Wix>
`
}

function esc(s: string): string {
  return s
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&apos;")
}
