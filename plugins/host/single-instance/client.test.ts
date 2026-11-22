import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { installTyndMock, type TyndMock } from "@tynd/core/testing";
import { singleInstance } from "./client";

let bridge: TyndMock;
beforeEach(() => {
  bridge = installTyndMock({ osFallback: () => null });
});
afterEach(() => bridge.restore());

describe("@tynd/plugin-single-instance client", () => {
  test("acquire", async () => {
    await singleInstance.acquire("com.example.app");
    expect(bridge.calls).toEqual([
      { kind: "os", api: "singleInstance", method: "acquire", args: { id: "com.example.app" } },
    ]);
  });

  test("isAcquired", async () => {
    await singleInstance.isAcquired();
    expect(bridge.calls).toEqual([
      { kind: "os", api: "singleInstance", method: "isAcquired", args: null },
    ]);
  });
});
