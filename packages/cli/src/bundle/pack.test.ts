import { afterEach, beforeEach, describe, expect, test } from "bun:test"
import { mkdtempSync, rmSync, writeFileSync } from "node:fs"
import { tmpdir } from "node:os"
import path from "node:path"

// packAssets isn't exported. Exercise it via the only way that reaches it
// from the public surface: packageLite with a tiny host + bundle fixture.
import { packageLite } from "./pack.ts"

let dir: string

beforeEach(() => {
  dir = mkdtempSync(path.join(tmpdir(), "tynd-pack-"))
})
afterEach(() => {
  rmSync(dir, { recursive: true, force: true })
})

describe("packageLite — TYNDPKG wire format", () => {
  test("appends magic at the end of the host binary", async () => {
    const hostBin = path.join(dir, "host.bin")
    const bundle = path.join(dir, "bundle.js")
    writeFileSync(hostBin, Buffer.from("HOSTBINARY"))
    writeFileSync(bundle, "export default 1")

    const out = path.join(dir, "out.bin")
    await packageLite({
      hostBin,
      backendBundle: bundle,
      frontendFiles: [],
      outFile: out,
      platform: "linux",
      iconPath: null,
    })

    const buf = Bun.file(out).bytes()
    const bytes = await buf
    const tail = Buffer.from(bytes.slice(bytes.length - 8)).toString("ascii")
    expect(tail).toBe("TYNDPKG\0")
  })

  test("section size trailer points inside the file", async () => {
    const hostBin = path.join(dir, "host.bin")
    const bundle = path.join(dir, "bundle.js")
    writeFileSync(hostBin, Buffer.alloc(64, 0xaa))
    writeFileSync(bundle, "x")

    const out = path.join(dir, "out.bin")
    await packageLite({
      hostBin,
      backendBundle: bundle,
      frontendFiles: [],
      outFile: out,
      platform: "linux",
      iconPath: null,
    })

    const bytes = Buffer.from(await Bun.file(out).bytes())
    const sizeOffset = bytes.length - 16
    const sectionSize = Number(bytes.readBigUInt64LE(sizeOffset))
    // Section starts just after the host and ends at the trailer. Size must
    // equal (trailer offset) - (post-host offset).
    const postHostLen = bytes.length - 64 - 16
    expect(sectionSize).toBe(postHostLen)
  })

  test("host bytes survive untouched", async () => {
    const hostBin = path.join(dir, "host.bin")
    const bundle = path.join(dir, "bundle.js")
    const hostBytes = Buffer.from("tynd-host-stub")
    writeFileSync(hostBin, hostBytes)
    writeFileSync(bundle, "y")

    const out = path.join(dir, "out.bin")
    await packageLite({
      hostBin,
      backendBundle: bundle,
      frontendFiles: [],
      outFile: out,
      platform: "linux",
      iconPath: null,
    })

    const bytes = Buffer.from(await Bun.file(out).bytes())
    expect(bytes.subarray(0, hostBytes.length).toString()).toBe("tynd-host-stub")
  })
})
