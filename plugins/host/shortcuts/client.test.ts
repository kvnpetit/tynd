import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { installTyndMock, type TyndMock } from "@tynd/core/testing";
import { shortcuts } from "./client";

let bridge: TyndMock;
beforeEach(() => {
  bridge = installTyndMock({ osFallback: () => null });
});
afterEach(() => bridge.restore());

describe("@tynd/plugin-shortcuts client", () => {
  test("isRegistered", async () => {
    await shortcuts.isRegistered("a");
    expect(bridge.calls).toEqual([
      { kind: "os", api: "shortcuts", method: "isRegistered", args: { id: "a" } },
    ]);
  });

  test("unregisterAll", async () => {
    await shortcuts.unregisterAll();
    expect(bridge.calls).toEqual([
      { kind: "os", api: "shortcuts", method: "unregisterAll", args: null },
    ]);
  });
});
