import { describe, expect, test } from "bun:test"
import { path } from "./path.ts"

describe("path", () => {
  test("join filters empty, normalises separators", () => {
    expect(path.join("a", "", "b", "c")).toMatch(/^a[/\\]b[/\\]c$/)
    expect(path.join("a/", "/b", "c")).toMatch(/^a[/\\]b[/\\]c$/)
  })

  test("dirname strips trailing separator", () => {
    expect(path.dirname("/a/b/c")).toBe("/a/b")
    expect(path.dirname("a/b/")).toBe("a")
    expect(path.dirname("single")).toBe("")
  })

  test("basename + ext", () => {
    expect(path.basename("/a/b/c.txt")).toBe("c.txt")
    expect(path.basename("/a/b/c.txt", ".txt")).toBe("c")
    expect(path.basename("no-ext")).toBe("no-ext")
  })

  test("extname leading-dot ignored", () => {
    expect(path.extname("foo.ts")).toBe(".ts")
    expect(path.extname(".bashrc")).toBe("")
    expect(path.extname("archive.tar.gz")).toBe(".gz")
    expect(path.extname("no-ext")).toBe("")
  })
})
