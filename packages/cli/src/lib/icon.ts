import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs"
import path from "node:path"
import { log } from "./logger.ts"

// Ordered by preference: ICO best for Windows PE, then PNG, then SVG.
const ICON_CANDIDATES = [
  "public/favicon.ico",
  "public/favicon.png",
  "public/icon.ico",
  "public/icon.png",
  "public/logo.ico",
  "public/logo.png",
  "public/favicon.svg",
  "public/icon.svg",
  "public/logo.svg",
  "assets/icon.ico",
  "assets/icon.png",
  "icon.ico",
  "icon.png",
]

const RASTER_EXTS = new Set([".ico", ".png", ".jpg", ".jpeg", ".webp"])

/**
 * Find the best icon for the project, auto-converting SVG → PNG if needed.
 * Converted PNGs go to `.tynd/cache/` so bundlers like Vite don't pick them up.
 */
export async function detectIcon(cwd: string, configIcon?: string): Promise<string | null> {
  const candidates = configIcon
    ? [path.resolve(cwd, configIcon), ...ICON_CANDIDATES.map((r) => path.join(cwd, r))]
    : ICON_CANDIDATES.map((r) => path.join(cwd, r))

  let svgCandidate: string | null = null

  for (const abs of candidates) {
    if (!existsSync(abs)) continue
    const ext = path.extname(abs).toLowerCase()

    if (RASTER_EXTS.has(ext)) return abs

    if (ext === ".svg" && svgCandidate === null) {
      const base = abs.slice(0, -4)
      for (const rExt of [".ico", ".png", ".jpg"]) {
        const companion = base + rExt
        if (existsSync(companion)) return companion
      }
      svgCandidate = abs
    }
  }

  if (svgCandidate) {
    const outPath = path.join(cwd, ".tynd", "cache", "icon.png")
    const converted = await svgToPng(svgCandidate, outPath)
    if (converted) return converted
    log.warn(
      `Icon: ${path.basename(svgCandidate)} found but could not convert.\n` +
        `         → Add public/favicon.png (256×256 recommended)`,
    )
  }

  return null
}

/** Convert SVG → PNG via @resvg/resvg-js (WASM). Returns outPath on success. */
export async function svgToPng(svgPath: string, outPath: string): Promise<string | null> {
  try {
    const { Resvg } = await import("@resvg/resvg-js")
    mkdirSync(path.dirname(outPath), { recursive: true })
    const svg = readFileSync(svgPath, "utf8")
    const resvg = new Resvg(svg, {
      fitTo: { mode: "width", value: 256 },
      background: "transparent",
    })
    writeFileSync(outPath, resvg.render().asPng())
    return outPath
  } catch {
    return null
  }
}

/** Wrap raw PNG bytes in a minimal ICO container (Vista+ accepts PNG payloads). */
export function pngToIco(pngBytes: Buffer): Buffer {
  if (pngBytes.length < 24) throw new Error("Invalid PNG: too small")

  const pngWidth = pngBytes.readUInt32BE(16)
  const pngHeight = pngBytes.readUInt32BE(20)
  const icoW = pngWidth >= 256 ? 0 : pngWidth
  const icoH = pngHeight >= 256 ? 0 : pngHeight

  const dataOffset = 6 + 16 // ICONDIR + ICONDIRENTRY
  const buf = Buffer.allocUnsafe(dataOffset + pngBytes.length)

  buf.writeUInt16LE(0, 0)
  buf.writeUInt16LE(1, 2)
  buf.writeUInt16LE(1, 4)
  buf.writeUInt8(icoW, 6)
  buf.writeUInt8(icoH, 7)
  buf.writeUInt8(0, 8)
  buf.writeUInt8(0, 9)
  buf.writeUInt16LE(1, 10)
  buf.writeUInt16LE(32, 12)
  buf.writeUInt32LE(pngBytes.length, 14)
  buf.writeUInt32LE(dataOffset, 18)
  pngBytes.copy(buf, dataOffset)
  return buf
}

/** Embed an icon into a Windows PE binary. Non-fatal on error. */
export async function setWindowsExeIcon(
  exePath: string,
  iconPath: string,
  appName?: string,
): Promise<void> {
  try {
    const ResEdit = await import("resedit")

    const ext = path.extname(iconPath).toLowerCase()
    const icoData = ext === ".ico" ? readFileSync(iconPath) : pngToIco(readFileSync(iconPath))

    // ignoreCert is required for signed binaries (Bun ships signed).
    const exeData = readFileSync(exePath)
    const exe = ResEdit.NtExecutable.from(exeData, { ignoreCert: true })
    const res = ResEdit.NtExecutableResource.from(exe)
    const iconFile = ResEdit.Data.IconFile.from(icoData)

    const existing = ResEdit.Resource.IconGroupEntry.fromEntries(res.entries)
    const iconGroupID = existing[0]?.id ?? 1

    ResEdit.Resource.IconGroupEntry.replaceIconsForResource(
      res.entries,
      iconGroupID,
      0,
      iconFile.icons.map((item) => item.data),
    )

    // Rewrite VERSIONINFO so Task Manager shows appName instead of "bun".
    if (appName) {
      const versionInfos = ResEdit.Resource.VersionInfo.fromEntries(res.entries)
      for (const vi of versionInfos) {
        for (const lang of vi.getAllLanguagesForStringValues()) {
          vi.setStringValues(lang, {
            FileDescription: appName,
            ProductName: appName,
            InternalName: appName,
            OriginalFilename: `${appName}.exe`,
          })
        }
        vi.outputToResourceEntries(res.entries)
      }
    }

    res.outputResource(exe)
    writeFileSync(exePath, Buffer.from(exe.generate()))
  } catch (e) {
    log.warn(`.exe icon update failed: ${e}`)
  }
}
