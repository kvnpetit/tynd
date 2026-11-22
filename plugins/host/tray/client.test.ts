import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { installTyndMock, type TyndMock } from "@tynd/core/testing";
import { tray } from "./client";

let bridge: TyndMock;
beforeEach(() => {
  bridge = installTyndMock({ osFallback: () => null });
});
afterEach(() => bridge.restore());

describe("@tynd/plugin-tray client", () => {
  test("setTooltip", async () => {
    await tray.setTooltip("Hi");
    expect(bridge.calls).toEqual([
      { kind: "os", api: "tray", method: "setTooltip", args: { text: "Hi" } },
    ]);
  });

  test("setItemChecked", async () => {
    await tray.setItemChecked("mute", true);
    expect(bridge.calls).toEqual([
      { kind: "os", api: "tray", method: "setItemChecked", args: { id: "mute", checked: true } },
    ]);
  });
});
