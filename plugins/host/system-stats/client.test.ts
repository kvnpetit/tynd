import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { installTyndMock, type TyndMock } from "@tynd/core/testing";
import { systemStats } from "./client";

let bridge: TyndMock;
beforeEach(() => {
  bridge = installTyndMock({ osFallback: () => null });
});
afterEach(() => bridge.restore());

describe("@tynd/plugin-system-stats client", () => {
  test("cpu", async () => {
    await systemStats.cpu();
    expect(bridge.calls).toEqual([{ kind: "os", api: "systemStats", method: "cpu", args: {} }]);
  });

  test("memory", async () => {
    await systemStats.memory();
    expect(bridge.calls).toEqual([{ kind: "os", api: "systemStats", method: "memory", args: {} }]);
  });

  test("disks", async () => {
    await systemStats.disks();
    expect(bridge.calls).toEqual([{ kind: "os", api: "systemStats", method: "disks", args: {} }]);
  });
});
