import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { installTyndMock, type TyndMock } from "@tynd/core/testing";
import { autolaunch } from "./client";

let bridge: TyndMock;
beforeEach(() => {
  bridge = installTyndMock({ osFallback: () => null });
});
afterEach(() => bridge.restore());

describe("@tynd/plugin-autolaunch client", () => {
  test("enable", async () => {
    await autolaunch.enable({ name: "Demo" });
    expect(bridge.calls).toEqual([
      { kind: "os", api: "autolaunch", method: "enable", args: { name: "Demo" } },
    ]);
  });

  test("disable defaults to empty options", async () => {
    await autolaunch.disable();
    expect(bridge.calls).toEqual([{ kind: "os", api: "autolaunch", method: "disable", args: {} }]);
  });

  test("enable/disable resolve to the plain boolean the host returns", async () => {
    bridge.restore();
    bridge = installTyndMock({ osFallback: (_api, method) => method === "enable" });
    expect(await autolaunch.enable()).toBe(true);
    expect(await autolaunch.disable()).toBe(false);
  });

  test("isEnabled", async () => {
    await autolaunch.isEnabled();
    expect(bridge.calls).toEqual([
      { kind: "os", api: "autolaunch", method: "isEnabled", args: {} },
    ]);
  });
});
