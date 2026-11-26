import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { installTyndMock, type TyndMock } from "@tynd/core/testing";
import { menu } from "./client";

let bridge: TyndMock;
beforeEach(() => {
  bridge = installTyndMock({ osFallback: () => null });
});
afterEach(() => bridge.restore());

function osCalls(): [string, unknown][] {
  return bridge.calls.flatMap((call) =>
    call.kind === "os" ? [[call.method, call.args] as [string, unknown]] : [],
  );
}

function lastCall() {
  return bridge.calls.at(-1);
}

function lastArgs(): unknown {
  return osCalls().at(-1)?.[1];
}

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

  test("showContextMenu goes through the window API with typed items", async () => {
    await menu.showContextMenu([{ id: "a", label: "A" }], { x: 1, y: 2 });
    expect(bridge.calls).toEqual([
      {
        kind: "os",
        api: "window",
        method: "showContextMenu",
        args: { items: [{ id: "a", label: "A", type: "item" }], x: 1, y: 2 },
      },
    ]);
  });
});

describe("@tynd/plugin-menu item handles", () => {
  test("item handle routes every mutation by id", async () => {
    const save = menu.item("file.save");
    await save.setEnabled(false);
    await save.setLabel("Save As…");
    await save.setChecked(true);
    await save.setVisible(false);
    await save.setAccelerator(null);
    await save.remove();
    expect(osCalls()).toEqual([
      ["setItemEnabled", { id: "file.save", enabled: false }],
      ["setItemLabel", { id: "file.save", label: "Save As…" }],
      ["setItemChecked", { id: "file.save", checked: true }],
      ["setItemVisible", { id: "file.save", visible: false }],
      ["setItemAccelerator", { id: "file.save", accelerator: null }],
      ["remove", { id: "file.save" }],
    ]);
  });

  test("setIcon encodes bytes as base64 and accepts native icons", async () => {
    const item = menu.item("open");
    await item.setIcon({ rgba: new Uint8Array([255, 0, 0, 255]), width: 1, height: 1 });
    expect(lastArgs()).toEqual({
      id: "open",
      icon: { rgba: "/wAA/w==", width: 1, height: 1 },
      nativeIcon: null,
    });
    await item.setIcon({ native: "folder" });
    expect(lastArgs()).toEqual({ id: "open", icon: null, nativeIcon: "folder" });
    await item.setIcon(null);
    expect(lastArgs()).toEqual({ id: "open", icon: null, nativeIcon: null });
  });

  test("append fills in item types recursively", async () => {
    await menu
      .item("file")
      .append({ label: "Recent", items: [{ id: "recent.clear", label: "Clear" }] }, { index: 2 });
    expect(lastCall()).toEqual({
      kind: "os",
      api: "menu",
      method: "append",
      args: {
        parent: "file",
        index: 2,
        item: {
          type: "submenu",
          label: "Recent",
          items: [{ type: "item", id: "recent.clear", label: "Clear" }],
        },
      },
    });
  });
});

describe("@tynd/plugin-menu context menus and templates", () => {
  test("context menu registration and popup", async () => {
    await menu.registerContextMenu("row", [{ id: "row.delete", label: "Delete" }]);
    await menu.popupContextMenu("row", { data: "42" });
    await menu.unregisterContextMenu("row");
    expect(osCalls()).toEqual([
      [
        "registerContextMenu",
        { id: "row", items: [{ id: "row.delete", label: "Delete", type: "item" }] },
      ],
      ["popupContextMenu", { menu: "row", data: "42", x: undefined, y: undefined }],
      ["unregisterContextMenu", { id: "row" }],
    ]);
  });

  test("onClick passes the event payload, including context data", () => {
    const seen: unknown[] = [];
    const off = menu.onClick("row.delete", (event) => seen.push(event));
    bridge.emitOs("menu:action", { id: "other" });
    bridge.emitOs("menu:action", { id: "row.delete", contextMenu: "row", data: "42" });
    off();
    expect(seen).toEqual([{ id: "row.delete", contextMenu: "row", data: "42" }]);
  });

  test("defaultAppMenu follows the platform conventions", () => {
    const mac = menu.defaultAppMenu({ platform: "macos", appName: "Notes" });
    expect(mac.map((entry) => entry.label)).toEqual(["Notes", "File", "Edit", "View", "Window"]);
    expect(mac[0]?.items.at(-1)).toEqual({ type: "item", role: "quit" });

    const windows = menu.defaultAppMenu({ platform: "windows" });
    expect(windows.map((entry) => entry.id)).toEqual(["file", "edit", "window"]);

    const linux = menu.defaultAppMenu({ platform: "linux" });
    expect(linux.map((entry) => entry.id)).toEqual(["edit"]);
  });

  test("setMenu sends the whole bar", async () => {
    await menu.setMenu(menu.defaultAppMenu({ platform: "linux" }));
    expect(osCalls().at(-1)?.[0]).toBe("setMenu");
    const args = lastArgs() as { items: unknown[] } | undefined;
    expect(args?.items).toHaveLength(1);
  });
});
