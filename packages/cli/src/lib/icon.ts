import { existsSync } from "node:fs"
import path from "node:path"
import { log } from "./logger.ts"

// SVG first: single source of truth for multi-size rendering.
const ICON_CANDIDATES = [
  "public/favicon.svg",
  "public/icon.svg",
  "public/logo.svg",
  "public/favicon.ico",
  "public/favicon.png",
  "public/icon.ico",
  "public/icon.png",
  "public/logo.ico",
  "public/logo.png",
  "assets/icon.svg",
  "assets/icon.png",
  "assets/icon.ico",
  "icon.svg",
  "icon.png",
  "icon.ico",
]

export interface PngEntry {
  size: number
  data: Buffer
}

/** Locate the project icon. Returns the original source path (svg/png/ico). */
export function detectIcon(cwd: string, configIcon?: string): string | null {
  const candidates = configIcon
    ? [path.resolve(cwd, configIcon), ...ICON_CANDIDATES.map((r) => path.join(cwd, r))]
    : ICON_CANDIDATES.map((r) => path.join(cwd, r))

  for (const abs of candidates) {
    if (existsSync(abs)) return abs
  }
  return null
}

/**
 * Rewrite a non-square SVG to a square viewBox so rendered PNGs are square
 * (letterboxed with transparency). Windows PE icons and macOS ICNS reject or
 * distort non-square sources.
 */
export function squarifySvg(svg: string): string {
  const rootMatch = svg.match(/<svg\b[^>]*>/i)
  if (!rootMatch) return svg
  const rootTag = rootMatch[0]

  const vbMatch = rootTag.match(/viewBox\s*=\s*"([^"]+)"/i)
  const wMatch = rootTag.match(/\bwidth\s*=\s*"([^"]+)"/i)
  const hMatch = rootTag.match(/\bheight\s*=\s*"([^"]+)"/i)

  let vbX = 0
  let vbY = 0
  let vbW = 0
  let vbH = 0
  if (vbMatch?.[1]) {
    const p = vbMatch[1]
      .trim()
      .split(/[\s,]+/)
      .map(Number)
    if (p.length === 4 && p.every(Number.isFinite)) {
      ;[vbX, vbY, vbW, vbH] = p as [number, number, number, number]
    }
  }
  if (!vbW || !vbH) {
    vbW = wMatch ? Number.parseFloat(wMatch[1]!) : 0
    vbH = hMatch ? Number.parseFloat(hMatch[1]!) : 0
  }
  if (!vbW || !vbH || Math.abs(vbW - vbH) < 0.5) return svg

  const size = Math.max(vbW, vbH)
  const newVbX = vbX - (size - vbW) / 2
  const newVbY = vbY - (size - vbH) / 2
  const newViewBox = `${newVbX} ${newVbY} ${size} ${size}`

  let newRoot = vbMatch
    ? rootTag.replace(/viewBox\s*=\s*"[^"]+"/i, `viewBox="${newViewBox}"`)
    : rootTag.replace(/<svg\b/i, `<svg viewBox="${newViewBox}"`)
  newRoot = wMatch
    ? newRoot.replace(/\bwidth\s*=\s*"[^"]+"/i, `width="${size}"`)
    : newRoot.replace(/<svg\b/i, `<svg width="${size}"`)
  newRoot = hMatch
    ? newRoot.replace(/\bheight\s*=\s*"[^"]+"/i, `height="${size}"`)
    : newRoot.replace(/<svg\b/i, `<svg height="${size}"`)

  return svg.replace(rootTag, newRoot)
}

/**
 * Wrap one or more PNG entries in an ICO container (Vista+ accepts PNG payloads).
 * Entries ordered smallest-first by convention; Windows picks the best per-DPI.
 */
export function pngToIco(entries: readonly PngEntry[]): Buffer {
  if (entries.length === 0) throw new Error("pngToIco: entries is empty")

  const count = entries.length
  const headerSize = 6 + 16 * count
  let dataOffset = headerSize

  const header = Buffer.alloc(headerSize)
  header.writeUInt16LE(0, 0) // reserved
  header.writeUInt16LE(1, 2) // type: icon
  header.writeUInt16LE(count, 4)

  entries.forEach((e, i) => {
    if (e.data.length < 24) throw new Error("pngToIco: PNG entry too small")
    const off = 6 + 16 * i
    // ICO width/height are u8; 0 means 256+.
    const dim = e.size >= 256 ? 0 : e.size
    header.writeUInt8(dim, off) // width
    header.writeUInt8(dim, off + 1) // height
    header.writeUInt8(0, off + 2) // colorCount (0 = truecolor)
    header.writeUInt8(0, off + 3) // reserved
    header.writeUInt16LE(1, off + 4) // planes
    header.writeUInt16LE(32, off + 6) // bitCount
    header.writeUInt32LE(e.data.length, off + 8)
    header.writeUInt32LE(dataOffset, off + 12)
    dataOffset += e.data.length
  })

  return Buffer.concat([header, ...entries.map((e) => e.data)])
}

/** Embed pre-built ICO bytes into a Windows PE binary. Non-fatal on error. */
export async function setWindowsExeIcon(
  exePath: string,
  icoBytes: Buffer,
  appName?: string,
): Promise<void> {
  try {
    const ResEdit = await import("resedit")

    // ignoreCert is required for signed binaries (Bun ships signed).
    const exeData = Buffer.from(await Bun.file(exePath).bytes())
    const exe = ResEdit.NtExecutable.from(exeData, { ignoreCert: true })
    const res = ResEdit.NtExecutableResource.from(exe)
    const iconFile = ResEdit.Data.IconFile.from(icoBytes)

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
    await Bun.write(exePath, Buffer.from(exe.generate()))
  } catch (e) {
    log.warn(`.exe icon update failed: ${e}`)
  }
}
