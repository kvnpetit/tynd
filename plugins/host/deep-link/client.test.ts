import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { installTyndMock, type TyndMock } from "@tynd/core/testing";
import { deepLink } from "./client";

let bridge: TyndMock;
beforeEach(() => {
  bridge = installTyndMock({ osFallback: () => null });
});
afterEach(() => bridge.restore());

describe("@tynd/plugin-deep-link client", () => {
  test("register", async () => {
    await deepLink.register("demo");
    expect(bridge.calls).toEqual([
      { kind: "os", api: "deepLink", method: "register", args: { scheme: "demo" } },
    ]);
  });

  test("isRegistered", async () => {
    await deepLink.isRegistered("demo");
    expect(bridge.calls).toEqual([
      { kind: "os", api: "deepLink", method: "isRegistered", args: { scheme: "demo" } },
    ]);
  });

  test("getStartupUrl", async () => {
    await deepLink.getStartupUrl();
    expect(bridge.calls).toEqual([
      { kind: "os", api: "deepLink", method: "getStartupUrl", args: null },
    ]);
  });
});
