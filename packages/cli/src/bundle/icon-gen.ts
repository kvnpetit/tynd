import { readFileSync, writeFileSync } from "node:fs"
import path from "node:path"

// ICNS format: "icns" magic + u32 BE total_size, then entries of
// [type: 4 ASCII][entry_size: u32 BE][PNG bytes]. One entry is enough —
// macOS rescales missing sizes. Requires a square source.
export function generateIcns(pngBytes: Buffer, outPath: string): void {
  const { width, height } = readPngSize(pngBytes)
  if (width !== height) {
    throw new Error(`icon PNG must be square (got ${width}×${height})`)
  }
  const type = pickIcnsType(width)
  if (!type) {
    throw new Error(`icon PNG size ${width}×${width} unsupported (need ≥ 16)`)
  }

  const entrySize = 8 + pngBytes.length
  const totalSize = 8 + entrySize

  const buf = Buffer.allocUnsafe(totalSize)
  buf.write("icns", 0, 4, "ascii")
  buf.writeUInt32BE(totalSize, 4)
  buf.write(type, 8, 4, "ascii")
  buf.writeUInt32BE(entrySize, 12)
  pngBytes.copy(buf, 16)

  writeFileSync(outPath, buf)
}

export async function renderSvgToPng(svgPath: string, size: number): Promise<Buffer> {
  const { Resvg } = await import("@resvg/resvg-js")
  const svg = readFileSync(svgPath, "utf8")
  const resvg = new Resvg(svg, {
    fitTo: { mode: "width", value: size },
    background: "transparent",
  })
  return Buffer.from(resvg.render().asPng())
}

export async function loadIconAsPng(iconPath: string): Promise<Buffer> {
  const ext = path.extname(iconPath).toLowerCase()
  if (ext === ".svg") return renderSvgToPng(iconPath, 512)
  if (ext === ".png") return readFileSync(iconPath)
  throw new Error(`unsupported icon source for bundling: ${ext} (use .png or .svg)`)
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
