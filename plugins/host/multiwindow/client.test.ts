import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { installTyndMock, type TyndMock } from "@tynd/core/testing";
import { multiwindow } from "./client";

let bridge: TyndMock;
beforeEach(() => {
  bridge = installTyndMock({ osFallback: () => null });
});
afterEach(() => bridge.restore());

describe("@tynd/plugin-multiwindow client", () => {
  test("create", async () => {
    await multiwindow.create({ label: "settings", url: "/settings" });
    expect(bridge.calls).toEqual([
      {
        kind: "os",
        api: "window",
        method: "create",
        args: { label: "settings", url: "/settings" },
      },
    ]);
  });

  test("close", async () => {
    await multiwindow.close("settings");
    expect(bridge.calls).toEqual([
      { kind: "os", api: "window", method: "close", args: { label: "settings" } },
    ]);
  });

  test("webview.create passes newWindow and proxy through", async () => {
    await multiwindow.webview.create({
      label: "w",
      x: 0,
      y: 0,
      width: 10,
      height: 10,
      newWindow: "deny",
      proxy: "socks5://127.0.0.1:1080",
    });
    expect(bridge.calls).toEqual([
      {
        kind: "os",
        api: "window",
        method: "createWebview",
        args: {
          label: "w",
          x: 0,
          y: 0,
          width: 10,
          height: 10,
          newWindow: "deny",
          proxy: "socks5://127.0.0.1:1080",
        },
      },
    ]);
  });

  test("webview.setBounds flattens the rectangle", async () => {
    await multiwindow.webview.setBounds("w", { x: 1, y: 2, width: 3, height: 4 });
    expect(bridge.calls).toEqual([
      {
        kind: "os",
        api: "window",
        method: "setWebviewBounds",
        args: { label: "w", x: 1, y: 2, width: 3, height: 4 },
      },
    ]);
  });
});
