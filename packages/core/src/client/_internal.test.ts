import { describe, expect, test } from "bun:test"
import { base64ToBytes, bytesToBase64 } from "./_internal.ts"

describe("base64 helpers", () => {
  test("roundtrip preserves bytes", () => {
    const bytes = new Uint8Array([0, 1, 127, 128, 255, 0, 42])
    expect(base64ToBytes(bytesToBase64(bytes))).toEqual(bytes)
  })

  test("empty", () => {
    expect(bytesToBase64(new Uint8Array())).toBe("")
    expect(base64ToBytes("")).toEqual(new Uint8Array())
  })

  test("binary-safe on full byte range", () => {
    const bytes = new Uint8Array(256)
    for (let i = 0; i < 256; i++) bytes[i] = i
    expect(base64ToBytes(bytesToBase64(bytes))).toEqual(bytes)
  })
})
