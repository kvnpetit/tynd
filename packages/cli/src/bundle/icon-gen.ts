import path from "node:path"
import { type PngEntry, squarifySvg } from "../lib/icon.ts"
import { log } from "../lib/logger.ts"

/** Canonical sizes for multi-size Windows ICO (taskbar 16 -> Explorer 256). */
export const ICO_SIZES: readonly number[] = [16, 32, 48, 256]

/** Canonical sizes for multi-entry macOS ICNS (small -> Retina Dock 1024). */
export const ICNS_SIZES: readonly number[] = [32, 128, 256, 512, 1024]

/** Canonical size for AppImage root icon (single file, scaled by runtime). */
export const LINUX_APPIMAGE_SIZE = 512

/** Hicolor icon theme sizes for .deb / .rpm (apps/<size>x<size>/<name>.png). */
export const LINUX_HICOLOR_SIZES: readonly number[] = [16, 32, 48, 64, 128, 256, 512]

/**
 * Filter an icon source for a bundler that can only consume .svg/.png raster.
 * .ico sources are Windows-only; return null + warn rather than crash.
 */
export function rasterSource(source: string | null, bundleName: string): string | null {
  if (!source) return null
  if (path.extname(source).toLowerCase() === ".ico") {
    log.warn(`${bundleName}: icon source is .ico — skipped; use .svg or .png for this bundle`)
    return null
  }
  return source
}

export async function renderSvgToPng(svgPath: string, size: number): Promise<Buffer> {
  const { Resvg } = await import("@resvg/resvg-js")
  const svg = squarifySvg(await Bun.file(svgPath).text())
  const resvg = new Resvg(svg, {
    fitTo: { mode: "width", value: size },
    background: "transparent",
  })
  return Buffer.from(resvg.render().asPng())
}

/**
 * Render a set of PNGs for multi-size ICO / multi-entry ICNS.
 * - SVG source: rasterises at each requested size for pixel-perfect rendering.
 * - PNG source: returns a single entry with its native size (no upscale).
 * - ICO source: rejected — callers handle ICO pass-through before this.
 */
export async function renderIconPngSet(
  source: string,
  sizes: readonly number[],
): Promise<PngEntry[]> {
  const ext = path.extname(source).toLowerCase()

  if (ext === ".svg") {
    const { Resvg } = await import("@resvg/resvg-js")
    const svg = squarifySvg(await Bun.file(source).text())
    return sizes.map((size) => {
      const resvg = new Resvg(svg, {
        fitTo: { mode: "width", value: size },
        background: "transparent",
      })
      return { size, data: Buffer.from(resvg.render().asPng()) }
    })
  }

  if (ext === ".png") {
    const data = Buffer.from(await Bun.file(source).bytes())
    return [{ size: readPngSize(data).width, data }]
  }

  throw new Error(`renderIconPngSet: unsupported source "${ext}" — use .svg or .png`)
}

/** Single-size PNG for bundlers that embed one icon file (e.g. AppImage root). */
export async function loadIconAsPng(source: string, size = LINUX_APPIMAGE_SIZE): Promise<Buffer> {
  const ext = path.extname(source).toLowerCase()
  if (ext === ".svg") return renderSvgToPng(source, size)
  if (ext === ".png") return Buffer.from(await Bun.file(source).bytes())
  throw new Error(`loadIconAsPng: unsupported source "${ext}" — use .svg or .png`)
}

export interface HicolorEntry {
  size: number
  /** Relative path inside the filesystem root, e.g. "usr/share/icons/hicolor/256x256/apps/foo.png". */
  relPath: string
  data: Buffer
}

/**
 * Render the full hicolor icon theme (16..512) for .deb / .rpm.
 * PNG sources only yield their native size — SVG gives all sizes.
 */
export async function renderHicolorSet(source: string, appName: string): Promise<HicolorEntry[]> {
  const entries = await renderIconPngSet(source, LINUX_HICOLOR_SIZES)
  return entries.map((e) => ({
    size: e.size,
    relPath: `usr/share/icons/hicolor/${e.size}x${e.size}/apps/${appName}.png`,
    data: e.data,
  }))
}

/**
 * ICNS format: "icns" magic + u32 BE total_size, then entries of
 * [type: 4 ASCII][entry_size: u32 BE][PNG bytes]. One entry per size bucket.
 */
export async function generateIcns(entries: readonly PngEntry[], outPath: string): Promise<void> {
  if (entries.length === 0) throw new Error("generateIcns: entries is empty")

  const chunks: Buffer[] = []
  let totalSize = 8 // "icns" + size u32

  for (const e of entries) {
    const type = pickIcnsType(e.size)
    if (!type) continue // below 16 — ICNS has no type for it
    const entrySize = 8 + e.data.length
    const header = Buffer.alloc(8)
    header.write(type, 0, 4, "ascii")
    header.writeUInt32BE(entrySize, 4)
    chunks.push(header, Buffer.from(e.data))
    totalSize += entrySize
  }

  if (chunks.length === 0) {
    throw new Error(`generateIcns: no entry met the minimum size (16px)`)
  }

  const magic = Buffer.alloc(8)
  magic.write("icns", 0, 4, "ascii")
  magic.writeUInt32BE(totalSize, 4)

  await Bun.write(outPath, Buffer.concat([magic, ...chunks]))
}

function readPngSize(buf: Buffer): { width: number; height: number } {
  if (buf.length < 24) throw new Error("invalid PNG: too small")
  if (buf.readUInt32BE(0) !== 0x89504e47) throw new Error("not a PNG")
  return { width: buf.readUInt32BE(16), height: buf.readUInt32BE(20) }
}

// Canonical ICNS entry types per size, largest first.
function pickIcnsType(size: number): string | null {
  if (size >= 1024) return "ic10"
  if (size >= 512) return "ic09"
  if (size >= 256) return "ic08"
  if (size >= 128) return "ic07"
  if (size >= 64) return "icp6"
  if (size >= 32) return "icp5"
  if (size >= 16) return "icp4"
  return null
}
