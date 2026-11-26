import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { installTyndMock, type TyndMock } from "@tynd/core/testing";
import { tray } from "./client";

let bridge: TyndMock;
beforeEach(() => {
  bridge = installTyndMock({
    osFallback: (_api, method) => (method === "list" ? ["main", "status"] : null),
  });
});
afterEach(() => bridge.restore());

const os = (method: string, args: unknown) => ({ kind: "os" as const, api: "tray", method, args });

describe("@tynd/plugin-tray client", () => {
  test("top-level methods target the main tray", async () => {
    await tray.setTooltip("Hi");
    await tray.setItemChecked("mute", true);
    expect(bridge.calls).toEqual([
      os("setTooltip", { tray: "main", text: "Hi" }),
      os("setItemChecked", { tray: "main", id: "mute", checked: true }),
    ]);
  });

  test("create encodes byte icons and untyped menu items", async () => {
    const status = await tray.create("status", {
      icon: { rgba: new Uint8Array([1, 2, 3, 4]), width: 1, height: 1 },
      tooltip: "Status",
      menu: [
        { id: "open", label: "Open" },
        { label: "More", items: [{ id: "a", label: "A" }] },
      ],
    });
    expect(status.id).toBe("status");
    expect(bridge.calls[0]).toEqual(
      os("create", {
        tray: "status",
        tooltip: "Status",
        icon: { rgba: "AQIDBA==", width: 1, height: 1 },
        menu: [
          { id: "open", label: "Open", type: "item" },
          { label: "More", type: "submenu", items: [{ id: "a", label: "A", type: "item" }] },
        ],
      }),
    );
    await status.setIcon({ native: "add" }, { template: true });
    expect(bridge.calls[1]).toEqual(
      os("setIcon", { tray: "status", nativeIcon: "add", template: true }),
    );
  });
});

describe("@tynd/plugin-tray handles", () => {
  test("get, list and remove", async () => {
    expect((await tray.get("status"))?.id).toBe("status");
    expect(await tray.get("nope")).toBeNull();
    await tray.remove("status");
    await tray.remove();
    expect(bridge.calls.slice(-2)).toEqual([
      os("remove", { tray: "status" }),
      os("remove", { tray: "main" }),
    ]);
  });
});

describe("@tynd/plugin-tray menus, windows and events", () => {
  test("menu mutations and window placement", async () => {
    await tray.setItemIcon("open", null);
    await tray.setItemIcon("open", { png: new Uint8Array([0]) });
    await tray.append({ type: "radio", id: "r", label: "R" }, { parent: "more", index: 0 });
    await tray.attachWindow("popup");
    await tray.attachWindow({ hideOnBlur: false, gap: 4 });
    await tray.positionWindow();
    expect(bridge.calls).toEqual([
      os("setItemIcon", { tray: "main", id: "open" }),
      os("setItemIcon", { tray: "main", id: "open", icon: { png: "AA==" } }),
      os("append", {
        tray: "main",
        item: { type: "radio", id: "r", label: "R" },
        parent: "more",
        index: 0,
      }),
      os("attachWindow", { tray: "main", label: "popup" }),
      os("attachWindow", { tray: "main", label: "main", hideOnBlur: false, gap: 4 }),
      os("positionWindow", { tray: "main", label: "main" }),
    ]);
  });

  test("events are filtered by tray id", async () => {
    const status = await tray.create("status", { nativeIcon: "info" });
    const clicks: string[] = [];
    const menus: string[] = [];
    status.onClick((event) => clicks.push(`${event.tray}:${event.button}`));
    status.onMenu("open", (event) => menus.push(event.id));
    bridge.emitOs("tray:click", {
      tray: "main",
      button: "left",
      position: { x: 0, y: 0 },
      rect: null,
    });
    bridge.emitOs("tray:click", {
      tray: "status",
      button: "left",
      position: { x: 0, y: 0 },
      rect: null,
    });
    bridge.emitOs("menu:action", { id: "open", tray: "main" });
    bridge.emitOs("menu:action", { id: "open" });
    bridge.emitOs("menu:action", { id: "open", tray: "status" });
    expect(clicks).toEqual(["status:left"]);
    expect(menus).toEqual(["open"]);
  });
});
