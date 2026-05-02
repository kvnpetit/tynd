import { describe, expect, test } from "bun:test";
import { sanitizedMessage, sanitizeError } from "./error";

describe("sanitizeError", () => {
  test("Error instance — keeps name + message, drops stack", () => {
    const e = new TypeError("boom at /home/secret/path/file.ts:42");
    const out = sanitizeError(e);
    expect(out).toEqual({ name: "TypeError", message: "boom at /home/secret/path/file.ts:42" });
    // Critical security property: no stack ever leaks via the helper.
    expect(out).not.toHaveProperty("stack");
  });

  test("subclassed Error preserves the custom name", () => {
    class MyError extends Error {
      override name = "MyError";
    }
    const out = sanitizeError(new MyError("oops"));
    expect(out).toEqual({ name: "MyError", message: "oops" });
  });

  test("string thrown via `throw 'literal'` becomes a plain Error", () => {
    expect(sanitizeError("nope")).toEqual({ name: "Error", message: "nope" });
  });

  test("error-like object with message + name is honored", () => {
    expect(sanitizeError({ name: "WeirdError", message: "from-network" })).toEqual({
      name: "WeirdError",
      message: "from-network",
    });
  });

  test("plain object falls back to String() coercion", () => {
    expect(sanitizeError({ foo: "bar" })).toEqual({ name: "Error", message: "[object Object]" });
  });

  test("null / number / undefined coerce safely", () => {
    expect(sanitizeError(null)).toEqual({ name: "Error", message: "null" });
    expect(sanitizeError(42)).toEqual({ name: "Error", message: "42" });
    expect(sanitizeError(undefined)).toEqual({ name: "Error", message: "undefined" });
  });
});

describe("sanitizedMessage", () => {
  test("returns just the message string for ergonomic call sites", () => {
    expect(sanitizedMessage(new Error("hello"))).toBe("hello");
    expect(sanitizedMessage("plain")).toBe("plain");
    expect(sanitizedMessage(123)).toBe("123");
  });
});
