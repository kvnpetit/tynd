import { describe, expect, test } from "bun:test";
import { security } from "./security";

describe("security", () => {
  test("exposes only the read-only policy probes", () => {
    expect(Object.keys(security).sort()).toEqual(["isFsAllowed", "isHttpAllowed"]);
  });
});
