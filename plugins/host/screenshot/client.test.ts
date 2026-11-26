import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { installTyndMock, type TyndMock } from "@tynd/core/testing";
import { screenshot } from "./client";

let bridge: TyndMock;
beforeEach(() => {
  bridge = installTyndMock({ osFallback: () => null });
});
afterEach(() => bridge.restore());

describe("@tynd/plugin-screenshot client", () => {
  test("captureScreen defaults to monitor 0", async () => {
    await screenshot.captureScreen();
    expect(bridge.calls).toEqual([
      { kind: "os", api: "screenshot", method: "captureScreen", args: { monitorIndex: 0 } },
    ]);
  });

  test("captureWindow", async () => {
    await screenshot.captureWindow(5);
    expect(bridge.calls).toEqual([
      { kind: "os", api: "screenshot", method: "captureWindow", args: { windowId: 5 } },
    ]);
  });

  test("listMonitors", async () => {
    await screenshot.listMonitors();
    expect(bridge.calls).toEqual([
      { kind: "os", api: "screenshot", method: "listMonitors", args: {} },
    ]);
  });
});
