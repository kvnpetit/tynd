import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { installTyndMock, type TyndMock } from "@tynd/core/testing";
import { menu } from "./client";

let bridge: TyndMock;
beforeEach(() => {
  bridge = installTyndMock({ osFallback: () => null });
});
afterEach(() => bridge.restore());

describe("@tynd/plugin-menu client", () => {
  test("setItemEnabled", async () => {
    await menu.setItemEnabled("file.save", false);
    expect(bridge.calls).toEqual([
      {
        kind: "os",
        api: "menu",
        method: "setItemEnabled",
        args: { id: "file.save", enabled: false },
      },
    ]);
  });

  test("showContextMenu goes through the window API", async () => {
    await menu.showContextMenu([{ id: "a", label: "A" }], { x: 1, y: 2 });
    expect(bridge.calls).toEqual([
      {
        kind: "os",
        api: "window",
        method: "showContextMenu",
        args: { items: [{ id: "a", label: "A" }], x: 1, y: 2 },
      },
    ]);
  });
});
