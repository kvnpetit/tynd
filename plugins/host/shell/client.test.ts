import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { installTyndMock, type TyndMock } from "@tynd/core/testing";
import { shell } from "./client";

let bridge: TyndMock;
beforeEach(() => {
  bridge = installTyndMock({ osFallback: () => null });
});
afterEach(() => bridge.restore());

describe("@tynd/plugin-shell client", () => {
  test("openExternal", async () => {
    await shell.openExternal("https://example.com");
    expect(bridge.calls).toEqual([
      { kind: "os", api: "shell", method: "openExternal", args: "https://example.com" },
    ]);
  });
});
