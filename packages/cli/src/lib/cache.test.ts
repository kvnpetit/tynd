import { afterEach, beforeEach, describe, expect, test } from "bun:test"
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs"
import { tmpdir } from "node:os"
import path from "node:path"
import { hashSources, readCache, wipeIfStaleVersion, writeCache } from "./cache.ts"
import { VERSION } from "./version.ts"

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

describe("wipeIfStaleVersion", () => {
  test("writes a stamp file when cache is fresh", () => {
    wipeIfStaleVersion(dir)
    const stamp = path.join(dir, ".cli-version")
    expect(existsSync(stamp)).toBe(true)
    expect(readFileSync(stamp, "utf8").trim()).toBe(VERSION)
  })

  test("leaves current-version caches untouched", () => {
    wipeIfStaleVersion(dir)
    writeCache(dir, "frontend", { hash: "abc", updatedAt: 1 })
    wipeIfStaleVersion(dir)
    expect(readCache(dir, "frontend")).toEqual({ hash: "abc", updatedAt: 1 })
  })

  test("wipes everything when stamp mismatches", () => {
    // Simulate an older CLI having written the cache.
    mkdirSync(dir, { recursive: true })
    writeFileSync(path.join(dir, ".cli-version"), "0.0.0-stale")
    writeCache(dir, "backend", { hash: "old", updatedAt: 1 })
    expect(readCache(dir, "backend")).not.toBe(null)

    wipeIfStaleVersion(dir)
    expect(readCache(dir, "backend")).toBe(null)
    expect(readFileSync(path.join(dir, ".cli-version"), "utf8").trim()).toBe(VERSION)
  })

  test("preserves tools/ dir across version bump", () => {
    mkdirSync(path.join(dir, "tools", "nsis", "3.09"), { recursive: true })
    writeFileSync(path.join(dir, "tools", "nsis", "3.09", "makensis"), "bin")
    writeFileSync(path.join(dir, ".cli-version"), "0.0.0-stale")
    writeCache(dir, "backend", { hash: "old", updatedAt: 1 })

    wipeIfStaleVersion(dir)
    expect(existsSync(path.join(dir, "tools", "nsis", "3.09", "makensis"))).toBe(true)
    expect(readCache(dir, "backend")).toBe(null)
  })
})
