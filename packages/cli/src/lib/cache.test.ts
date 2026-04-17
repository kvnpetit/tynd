import { afterEach, beforeEach, describe, expect, test } from "bun:test"
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs"
import { tmpdir } from "node:os"
import path from "node:path"
import { hashSources, readCache, writeCache } from "./cache.ts"

let dir: string

beforeEach(() => {
  dir = mkdtempSync(path.join(tmpdir(), "tynd-cache-"))
})
afterEach(() => {
  rmSync(dir, { recursive: true, force: true })
})

describe("hashSources", () => {
  test("deterministic over same inputs", () => {
    const f = path.join(dir, "a.ts")
    writeFileSync(f, "hello")
    expect(hashSources([], [f])).toBe(hashSources([], [f]))
  })

  test("changes when content changes", () => {
    const f = path.join(dir, "a.ts")
    writeFileSync(f, "v1")
    const h1 = hashSources([], [f])
    writeFileSync(f, "v2")
    expect(hashSources([], [f])).not.toBe(h1)
  })

  test("missing files are ignored, not errors", () => {
    const a = path.join(dir, "a.ts")
    const missing = path.join(dir, "never.ts")
    writeFileSync(a, "x")
    expect(hashSources([], [a, missing])).toBe(hashSources([], [a]))
  })

  test("walks dirs, excludes node_modules / .tynd / target", () => {
    mkdirSync(path.join(dir, "src"))
    writeFileSync(path.join(dir, "src", "a.ts"), "a")
    mkdirSync(path.join(dir, "node_modules"))
    writeFileSync(path.join(dir, "node_modules", "ignored.ts"), "noise")

    const before = hashSources([dir], [])
    writeFileSync(path.join(dir, "node_modules", "ignored.ts"), "changed noise")
    expect(hashSources([dir], [])).toBe(before)
  })

  test("path contributes — rename changes hash", () => {
    const a = path.join(dir, "a.ts")
    const b = path.join(dir, "b.ts")
    writeFileSync(a, "same")
    const h1 = hashSources([], [a])
    writeFileSync(b, "same")
    expect(hashSources([], [b])).not.toBe(h1)
  })
})

describe("readCache / writeCache", () => {
  test("roundtrip", () => {
    writeCache(dir, "frontend", { hash: "abc", updatedAt: 123 })
    expect(readCache(dir, "frontend")).toEqual({ hash: "abc", updatedAt: 123 })
  })

  test("missing key returns null", () => {
    expect(readCache(dir, "nope")).toBe(null)
  })

  test("invalid schema returns null", () => {
    writeFileSync(path.join(dir, "bad.json"), JSON.stringify({ nope: true }))
    expect(readCache(dir, "bad")).toBe(null)
  })
})
