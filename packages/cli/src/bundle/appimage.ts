import {
  chmodSync,
  copyFileSync,
  existsSync,
  mkdirSync,
  rmSync,
  symlinkSync,
  writeFileSync,
} from "node:fs"
import path from "node:path"
import { exec } from "../lib/exec.ts"
import { log } from "../lib/logger.ts"
import { loadIconAsPng } from "./icon-gen.ts"
import { ensureTool, type ToolSpec } from "./tools.ts"
import type { BundleContext } from "./types.ts"

const APPIMAGETOOL: ToolSpec = {
  name: "appimagetool",
  version: "13",
  archive: "raw",
  binRel: "appimagetool",
  url: (platform, arch) => {
    if (platform !== "linux") return null
    const a = arch === "arm64" ? "aarch64" : "x86_64"
    return `https://github.com/AppImage/AppImageKit/releases/download/13/appimagetool-${a}.AppImage`
  },
}

export async function bundleAppImage(ctx: BundleContext): Promise<string> {
  if (ctx.platform !== "linux") {
    throw new Error(".AppImage bundles can only be built on a Linux host")
  }

  const tool = await ensureTool(APPIMAGETOOL, ctx.toolsDir, ctx.platform, ctx.arch)

  const appDir = path.join(ctx.outDir, `${ctx.appName}.AppDir`)
  if (existsSync(appDir)) rmSync(appDir, { recursive: true, force: true })
  mkdirSync(path.join(appDir, "usr", "bin"), { recursive: true })
  mkdirSync(path.join(appDir, "usr", "share", "icons", "hicolor", "256x256", "apps"), {
    recursive: true,
  })

  const binDest = path.join(appDir, "usr", "bin", ctx.appName)
  copyFileSync(ctx.inputBinary, binDest)
  chmodSync(binDest, 0o755)

  // appimagetool looks for AppRun at the AppDir root; a symlink is enough.
  symlinkSync(path.join("usr", "bin", ctx.appName), path.join(appDir, "AppRun"))

  writeFileSync(path.join(appDir, `${ctx.appName}.desktop`), renderDesktopEntry(ctx), {
    mode: 0o644,
  })

  if (ctx.iconSource) {
    const iconPng = await loadIconAsPng(ctx.iconSource)
    // Top-level PNG is what appimagetool embeds as the AppImage's own icon.
    writeFileSync(path.join(appDir, `${ctx.appName}.png`), iconPng, { mode: 0o644 })
    writeFileSync(
      path.join(
        appDir,
        "usr",
        "share",
        "icons",
        "hicolor",
        "256x256",
        "apps",
        `${ctx.appName}.png`,
      ),
      iconPng,
      { mode: 0o644 },
    )
  }

  const outFile = path.join(ctx.outDir, `${ctx.appName}-${ctx.version}-${ctx.arch}.AppImage`)
  if (existsSync(outFile)) rmSync(outFile, { force: true })

  log.step(`Running appimagetool…`)
  await exec(tool.bin, ["--no-appstream", appDir, outFile], {
    silent: true,
    env: { ARCH: ctx.arch === "arm64" ? "aarch64" : "x86_64" },
  })

  rmSync(appDir, { recursive: true, force: true })

  log.success(`AppImage → ${log.cyan(`release/${path.basename(outFile)}`)}`)
  return outFile
}

function renderDesktopEntry(ctx: BundleContext): string {
  const cats = ctx.categories.length > 0 ? `${ctx.categories.join(";")};` : "Utility;"
  return [
    "[Desktop Entry]",
    "Type=Application",
    `Name=${ctx.displayName}`,
    `Comment=${ctx.shortDescription}`,
    `Exec=${ctx.appName}`,
    `Icon=${ctx.appName}`,
    "Terminal=false",
    `Categories=${cats}`,
    "",
  ].join("\n")
}
