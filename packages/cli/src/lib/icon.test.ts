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
  // Minimal 1x1 PNG fixture so tests don't depend on a real image on disk.
  function makePng(width: number, height: number): Buffer {
    const buf = Buffer.alloc(24)
    buf.writeUInt32BE(0x89504e47, 0)
    buf.writeUInt32LE(0x0a1a0a0d, 4)
    buf.writeUInt32BE(width, 16)
    buf.writeUInt32BE(height, 20)
    return buf
  }

  test("ICO header magic + one entry", () => {
    const ico = pngToIco(makePng(64, 64))
    expect(ico.readUInt16LE(0)).toBe(0)
    expect(ico.readUInt16LE(2)).toBe(1)
    expect(ico.readUInt16LE(4)).toBe(1)
    expect(ico.readUInt8(6)).toBe(64)
    expect(ico.readUInt8(7)).toBe(64)
  })

  test("256×256 stores 0/0 (ICO convention)", () => {
    const ico = pngToIco(makePng(256, 256))
    expect(ico.readUInt8(6)).toBe(0)
    expect(ico.readUInt8(7)).toBe(0)
  })

  test("rejects too-small buffer", () => {
    expect(() => pngToIco(Buffer.alloc(10))).toThrow()
  })
})
