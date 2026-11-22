import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { installTyndMock, type MockOsCall, type TyndMock } from "../testing";
import { tyndWindow } from "./window";

let bridge: TyndMock;
beforeEach(() => {
  bridge = installTyndMock({ osFallback: () => null });
});
afterEach(() => bridge.restore());

const osCalls = () => bridge.calls.filter((call): call is MockOsCall => call.kind === "os");

describe("tyndWindow.setEffects", () => {
  test("forwards the effect list and options", async () => {
    await tyndWindow.setEffects({ effects: ["mica", "sidebar"], state: "active", radius: 8 });
    await tyndWindow.setEffects({ effects: [] });
    expect(osCalls().map(({ method, args }) => ({ method, args }))).toMatchObject([
      { method: "setEffects", args: { effects: ["mica", "sidebar"], state: "active", radius: 8 } },
      { method: "setEffects", args: { effects: [] } },
    ]);
  });
});
