import { describe, expect, test } from "bun:test"
import { pngToIco, squarifySvg } from "./icon.ts"

describe("squarifySvg", () => {
  test("passthrough when already square", () => {
    const svg = `<svg viewBox="0 0 48 48" width="48" height="48"><rect/></svg>`
    expect(squarifySvg(svg)).toBe(svg)
  })

  test("wraps non-square viewBox into square", () => {
    const svg = `<svg viewBox="0 0 48 46" width="48" height="46"><rect/></svg>`
    const out = squarifySvg(svg)
    expect(out).toContain(`viewBox="0 -1 48 48"`)
    expect(out).toContain(`width="48"`)
    expect(out).toContain(`height="48"`)
  })

  test("uses max dimension as square size", () => {
    const svg = `<svg viewBox="0 0 100 40"><rect/></svg>`
    const out = squarifySvg(svg)
    expect(out).toMatch(/viewBox="0 -30 100 100"/)
  })

  test("no root <svg> tag returns unchanged", () => {
    expect(squarifySvg("<not-svg/>")).toBe("<not-svg/>")
  })
})

describe("pngToIco", () => {
  // Minimal PNG header fixture (24 bytes) — just enough for pngToIco to read the size.
  function makePng(width: number, height: number): Buffer {
    const buf = Buffer.alloc(24)
    buf.writeUInt32BE(0x89504e47, 0)
    buf.writeUInt32LE(0x0a1a0a0d, 4)
    buf.writeUInt32BE(width, 16)
    buf.writeUInt32BE(height, 20)
    return buf
  }

  test("single-entry ICO has one ICONDIRENTRY", () => {
    const ico = pngToIco([{ size: 64, data: makePng(64, 64) }])
    expect(ico.readUInt16LE(0)).toBe(0) // reserved
    expect(ico.readUInt16LE(2)).toBe(1) // type: icon
    expect(ico.readUInt16LE(4)).toBe(1) // count
    expect(ico.readUInt8(6)).toBe(64) // width
    expect(ico.readUInt8(7)).toBe(64) // height
  })

  test("256×256 stores 0/0 per ICO convention", () => {
    const ico = pngToIco([{ size: 256, data: makePng(256, 256) }])
    expect(ico.readUInt8(6)).toBe(0)
    expect(ico.readUInt8(7)).toBe(0)
  })

  test("multi-entry ICO declares count and ordered offsets", () => {
    const entries = [
      { size: 16, data: makePng(16, 16) },
      { size: 32, data: makePng(32, 32) },
      { size: 48, data: makePng(48, 48) },
      { size: 256, data: makePng(256, 256) },
    ]
    const ico = pngToIco(entries)

    expect(ico.readUInt16LE(4)).toBe(4) // count

    // ICONDIRENTRY at +6, +22, +38, +54 — sizes encoded as 16/32/48/0.
    expect(ico.readUInt8(6)).toBe(16)
    expect(ico.readUInt8(22)).toBe(32)
    expect(ico.readUInt8(38)).toBe(48)
    expect(ico.readUInt8(54)).toBe(0)

    // First data offset sits right after ICONDIR (6) + 4×ICONDIRENTRY (64) = 70.
    expect(ico.readUInt32LE(6 + 12)).toBe(70)

    // Each subsequent offset = previous offset + previous entry's data length.
    const len0 = ico.readUInt32LE(6 + 8)
    expect(ico.readUInt32LE(22 + 12)).toBe(70 + len0)
  })

  test("rejects empty entries", () => {
    expect(() => pngToIco([])).toThrow()
  })

  test("rejects PNG entry smaller than 24 bytes", () => {
    expect(() => pngToIco([{ size: 32, data: Buffer.alloc(10) }])).toThrow()
  })
})
