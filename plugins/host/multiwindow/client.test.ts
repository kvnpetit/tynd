import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { tyndWindow } from "@tynd/core/client";
import { createWindowHandle } from "@tynd/core/client/bridge";
import { installTyndMock, type TyndMock } from "@tynd/core/testing";
import { multiwindow } from "./client";

let bridge: TyndMock;
beforeEach(() => {
  bridge = installTyndMock({ osFallback: () => null });
});
afterEach(() => bridge.restore());

describe("multiwindow", () => {
  test("create forwards the shared window options and returns a handle", async () => {
    const win = await multiwindow.create({
      label: "settings",
      url: "/settings",
      x: 10,
      y: 20,
      visible: false,
      theme: "dark",
      skipTaskbar: true,
      backgroundColor: { r: 0, g: 0, b: 0 },
      kiosk: false,
      titleBarStyle: "overlay",
    });
    await win.show();
    await win.setOpacity(0.5);
    expect(bridge.calls).toEqual([
      {
        kind: "os",
        api: "window",
        method: "create",
        args: {
          label: "settings",
          url: "/settings",
          x: 10,
          y: 20,
          visible: false,
          theme: "dark",
          skipTaskbar: true,
          backgroundColor: { r: 0, g: 0, b: 0 },
          kiosk: false,
          titleBarStyle: "overlay",
        },
      },
      { kind: "os", api: "window", method: "show", args: { window: "settings" } },
      {
        kind: "os",
        api: "window",
        method: "setOpacity",
        args: { opacity: 0.5, window: "settings" },
      },
    ]);
    expect(win.label()).toBe("settings");
  });
});

describe("multiwindow", () => {
  test("getWindow / getAllWindows / getCurrentWindow", async () => {
    bridge.setOs("window", "all", () => ["main", "settings"]);
    expect(await multiwindow.getWindow("nope")).toBeNull();
    const settings = await multiwindow.getWindow("settings");
    await settings?.setTitle("Prefs");
    const all = await multiwindow.getAllWindows();
    expect(all.map((w) => w.label())).toEqual(["main", "settings"]);
    expect(all[0]).toBe(tyndWindow);
    expect(multiwindow.getCurrentWindow()).toBe(tyndWindow);
    const setTitle = bridge.calls.find((call) => call.kind === "os" && call.method === "setTitle");
    expect(setTitle).toEqual({
      kind: "os",
      api: "window",
      method: "setTitle",
      args: { title: "Prefs", window: "settings" },
    });
  });
});

describe("multiwindow", () => {
  test("handle events are filtered by the handle's label", () => {
    const seen: string[] = [];
    const settings = createWindowHandle("settings");
    const off = settings.onFocused(() => seen.push("settings"));
    tyndWindow.onFocused(() => seen.push("main"));
    bridge.emitOs("window:focused", { label: "settings" });
    bridge.emitOs("window:focused", { label: "main" });
    off();
    bridge.emitOs("window:focused", { label: "settings" });
    expect(seen).toEqual(["settings", "main"]);
  });
});

describe("multiwindow", () => {
  test("handle close, setMenu and startDrag", async () => {
    const settings = createWindowHandle("settings");
    await settings.close();
    await settings.setMenu([{ type: "submenu", label: "File", items: [] }]);
    await settings.setMenu(null);
    await tyndWindow.startDrag({ files: ["/tmp/a.txt"] });
    await tyndWindow.startDrag({ files: ["/tmp/a.txt"], icon: "/tmp/i.png" });
    await tyndWindow.startDrag({ files: ["/tmp/a.txt"], icon: new Uint8Array([1, 2]) });
    expect(
      bridge.calls.map((call) => (call.kind === "os" ? [call.method, call.args] : [])),
    ).toEqual([
      ["close", { label: "settings" }],
      ["setMenu", { items: [{ type: "submenu", label: "File", items: [] }], window: "settings" }],
      ["setMenu", { items: null, window: "settings" }],
      ["startDrag", { files: ["/tmp/a.txt"] }],
      ["startDrag", { files: ["/tmp/a.txt"], iconPath: "/tmp/i.png" }],
      ["startDrag", { files: ["/tmp/a.txt"], iconBytes: [1, 2] }],
    ]);
  });
});

describe("multiwindow", () => {
  test("findInPage refuses another window's handle", () => {
    expect(() => createWindowHandle("settings").findInPage("x")).toThrow("only searches");
  });
});

describe("multiwindow", () => {
  test("typed window messages", async () => {
    const bus = multiwindow.events<{ "doc:saved": { id: string }; other: number }>();
    const got: unknown[] = [];
    const off = bus.listen("doc:saved", (payload, meta) => got.push([payload, meta.from]));
    bridge.emitOs("multiwindow:message", { event: "other", payload: 1, from: "main" });
    bridge.emitOs("multiwindow:message", {
      event: "doc:saved",
      payload: { id: "42" },
      from: "editor",
    });
    off();
    expect(got).toEqual([[{ id: "42" }, "editor"]]);

    await bus.emitTo("main", "doc:saved", { id: "1" });
    await bus.emit("other", 2);
    expect(bridge.calls).toEqual([
      {
        kind: "os",
        api: "window",
        method: "emitTo",
        args: { to: "main", event: "doc:saved", payload: { id: "1" } },
      },
      { kind: "os", api: "window", method: "emit", args: { event: "other", payload: 2 } },
    ]);
  });
});

describe("multiwindow", () => {
  test("close", async () => {
    await multiwindow.close("settings");
    expect(bridge.calls).toEqual([
      { kind: "os", api: "window", method: "close", args: { label: "settings" } },
    ]);
  });
});

describe("multiwindow", () => {
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
});

describe("multiwindow", () => {
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
