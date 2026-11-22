import { describe, expect, test } from "bun:test";
import { decodeArgs, encode } from "./wire";

describe("wire", () => {
  test("round-trips the rich types devalue supports", () => {
    const value = [
      new Date(0),
      new Map([[1, 2]]),
      new Set(["a"]),
      1n,
      /x/g,
      new URL("https://a.b/c"),
    ];
    const [date, map, set, big, re, url] = decodeArgs(encode(value));
    expect(date).toEqual(new Date(0));
    expect(map).toEqual(new Map([[1, 2]]));
    expect(set).toEqual(new Set(["a"]));
    expect(big).toBe(1n);
    expect(String(re)).toBe("/x/g");
    expect(String(url)).toBe("https://a.b/c");
  });

  test("Error values are not encodable (handlers must throw instead)", () => {
    expect(() => encode(new Error("boom"))).toThrow();
    expect(() => encode({ error: new TypeError("boom") })).toThrow();
  });
});
