import { chmodSync, copyFileSync, existsSync, mkdirSync, rmSync } from "node:fs"
import path from "node:path"
import { log } from "../lib/logger.ts"
import { generateIcns, loadIconAsPng } from "./icon-gen.ts"
import type { BundleContext } from "./types.ts"

export async function bundleApp(ctx: BundleContext): Promise<string> {
  const appPath = path.join(ctx.outDir, `${ctx.displayName}.app`)
  if (existsSync(appPath)) rmSync(appPath, { recursive: true, force: true })

  const contents = path.join(appPath, "Contents")
  const macos = path.join(contents, "MacOS")
  const resources = path.join(contents, "Resources")
  mkdirSync(macos, { recursive: true })
  mkdirSync(resources, { recursive: true })

  const exeName = ctx.displayName
  const exeDest = path.join(macos, exeName)
  copyFileSync(ctx.inputBinary, exeDest)
  chmodSync(exeDest, 0o755)

  await Bun.write(path.join(contents, "PkgInfo"), "APPL????")

  let iconFile: string | null = null
  if (ctx.iconSource) {
    iconFile = "icon.icns"
    const pngBytes = await loadIconAsPng(ctx.iconSource)
    await generateIcns(pngBytes, path.join(resources, iconFile))
  }

  await Bun.write(path.join(contents, "Info.plist"), renderInfoPlist(ctx, exeName, iconFile))

  log.success(`App bundle -> ${log.cyan(`release/${path.basename(appPath)}`)}`)
  return appPath
}

function renderInfoPlist(ctx: BundleContext, exeName: string, iconFile: string | null): string {
  const category = ctx.categories[0] ?? "public.app-category.utilities"
  const copyright = esc(ctx.copyright)
  const iconKey = iconFile
    ? `    <key>CFBundleIconFile</key>\n    <string>${esc(iconFile)}</string>\n`
    : ""

  return `<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key>
    <string>${esc(ctx.displayName)}</string>
    <key>CFBundleDisplayName</key>
    <string>${esc(ctx.displayName)}</string>
    <key>CFBundleIdentifier</key>
    <string>${esc(ctx.identifier)}</string>
    <key>CFBundleVersion</key>
    <string>${esc(ctx.version)}</string>
    <key>CFBundleShortVersionString</key>
    <string>${esc(ctx.version)}</string>
    <key>CFBundleExecutable</key>
    <string>${esc(exeName)}</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleInfoDictionaryVersion</key>
    <string>6.0</string>
    <key>CFBundleSignature</key>
    <string>????</string>
    <key>LSApplicationCategoryType</key>
    <string>${esc(category)}</string>
    <key>LSMinimumSystemVersion</key>
    <string>10.13</string>
    <key>NSHighResolutionCapable</key>
    <true/>
    <key>NSHumanReadableCopyright</key>
    <string>${copyright}</string>
${iconKey}</dict>
</plist>
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
