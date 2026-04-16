/**
 * Icon utilities for vorn build — fully cross-platform (pure TS/JS + WASM).
 *
 * Conversion pipeline:
 *   SVG  → PNG  via @resvg/resvg-js (WASM, no native modules)
 *   PNG  → ICO  via pngToIco (pure Buffer manipulation)
 *   ICO/PNG → PE  via resedit (pure JS PE resource editor)
 */

import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs"
import path from "node:path"
import { log } from "./logger.ts"

// Ordered by preference: ICO best for Windows PE, then PNG, then SVG
const ICON_CANDIDATES = [
  "public/favicon.ico",
  "public/favicon.png",
  "public/icon.ico",
  "public/icon.png",
  "public/logo.ico",
  "public/logo.png",
  "public/favicon.svg", // SVG — auto-converted if found
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
 * Returns absolute path to a raster icon file, or null if nothing found.
 *
 * Converted PNGs are written to `.vorn/cache/` (never inside `public/`)
 * so Vite and other bundlers never pick them up as static assets.
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
      // Check for companion raster (favicon.svg → favicon.png / favicon.ico)
      const base = abs.slice(0, -4)
      for (const rExt of [".ico", ".png", ".jpg"]) {
        const companion = base + rExt
        if (existsSync(companion)) return companion
      }
      svgCandidate = abs
    }
  }

  // Convert SVG → PNG using @resvg/resvg-js (pure WASM, cross-platform).
  // Output goes to .vorn/cache/ — never inside public/ so bundlers ignore it.
  if (svgCandidate) {
    const cacheDir = path.join(cwd, ".vorn", "cache")
    const outPath = path.join(cacheDir, "icon.png")
    const converted = await svgToPng(svgCandidate, outPath)
    if (converted) return converted
    log.warn(
      `Icon: ${path.basename(svgCandidate)} found but could not convert.\n` +
        `         → Add public/favicon.png (256×256 recommended)`,
    )
  }

  return null
}

/**
 * Convert an SVG file to PNG using @resvg/resvg-js (WASM).
 * Writes the result to `outPath` (caller decides where — use .vorn/cache/).
 * Returns `outPath` on success, or null on failure.
 */
export async function svgToPng(svgPath: string, outPath: string): Promise<string | null> {
  try {
    const { Resvg } = await import("@resvg/resvg-js")
    mkdirSync(path.dirname(outPath), { recursive: true })
    const svg = readFileSync(svgPath, "utf8")
    const resvg = new Resvg(svg, {
      fitTo: { mode: "width", value: 256 },
      background: "transparent",
    })
    const rendered = resvg.render()
    const pngData = rendered.asPng()
    writeFileSync(outPath, pngData)
    return outPath
  } catch (_e) {
    return null
  }
}

/**
 * Wrap raw PNG bytes in a minimal ICO container.
 * Modern Windows (Vista+) supports PNG payloads inside ICO files directly.
 */
export function pngToIco(pngBytes: Buffer): Buffer {
  if (pngBytes.length < 24) throw new Error("Invalid PNG: too small")

  const pngWidth = pngBytes.readUInt32BE(16)
  const pngHeight = pngBytes.readUInt32BE(20)
  const icoW = pngWidth >= 256 ? 0 : pngWidth
  const icoH = pngHeight >= 256 ? 0 : pngHeight

  const dataOffset = 6 + 16 // ICONDIR + ICONDIRENTRY
  const buf = Buffer.allocUnsafe(dataOffset + pngBytes.length)

  buf.writeUInt16LE(0, 0) // reserved
  buf.writeUInt16LE(1, 2) // type: icon
  buf.writeUInt16LE(1, 4) // count: 1 entry
  buf.writeUInt8(icoW, 6)
  buf.writeUInt8(icoH, 7)
  buf.writeUInt8(0, 8) // color count
  buf.writeUInt8(0, 9) // reserved
  buf.writeUInt16LE(1, 10) // planes
  buf.writeUInt16LE(32, 12) // bit count
  buf.writeUInt32LE(pngBytes.length, 14)
  buf.writeUInt32LE(dataOffset, 18)
  pngBytes.copy(buf, dataOffset)
  return buf
}

/**
 * Embed an icon into a Windows PE binary using `resedit` (pure JS/TS).
 * Works cross-platform (but only useful for .exe targets of course).
 *
 * Accepts PNG (auto-wrapped in ICO) or ICO.
 * Non-fatal on any error.
 */
export async function setWindowsExeIcon(
  exePath: string,
  iconPath: string,
  appName?: string,
): Promise<void> {
  try {
    const ResEdit = await import("resedit")

    // Resolve ICO data
    const ext = path.extname(iconPath).toLowerCase()
    let icoData: Buffer
    if (ext === ".ico") {
      icoData = readFileSync(iconPath)
    } else {
      // PNG / JPG / etc. → wrap in ICO container
      icoData = pngToIco(readFileSync(iconPath))
    }

    // Parse the PE executable (ignoreCert needed for signed binaries like Bun's)
    const exeData = readFileSync(exePath)
    const exe = ResEdit.NtExecutable.from(exeData, { ignoreCert: true })
    const res = ResEdit.NtExecutableResource.from(exe)

    // Parse the ICO file into individual icon images
    const iconFile = ResEdit.Data.IconFile.from(icoData)

    // Find existing icon group ID or use 1
    const existing = ResEdit.Resource.IconGroupEntry.fromEntries(res.entries)
    const iconGroupID = existing[0]?.id ?? 1

    // Replace (or create) icon group in the resource section
    ResEdit.Resource.IconGroupEntry.replaceIconsForResource(
      res.entries,
      iconGroupID,
      0, // language
      iconFile.icons.map((item) => item.data),
    )

    // Patch version info so Task Manager shows appName instead of "bun"
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
    const updated = Buffer.from(exe.generate())
    writeFileSync(exePath, updated)
  } catch (e) {
    log.warn(`.exe icon update failed: ${e}`)
  }
}
