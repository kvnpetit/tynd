import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { menu } from "./menu";

type OsCall = { api: string; method: string; args: unknown };

function mountShim(): { osCalls: OsCall[] } {
  const osCalls: OsCall[] = [];
  (globalThis as unknown as { window: unknown }).window = {
    __tynd__: {
      os_call(api: string, method: string, args: unknown) {
        osCalls.push({ api, method, args });
        return Promise.resolve(null);
      },
      os_on() {
        return () => undefined;
      },
      call() {
        return Promise.resolve(null);
      },
      on() {
        return () => undefined;
      },
      off() {
        return undefined;
      },
    },
  };
  return { osCalls };
}

describe("menu mutation API — wire format", () => {
  let osCalls: OsCall[];
  beforeEach(() => {
    ({ osCalls } = mountShim());
  });
  afterEach(() => {
    (globalThis as { window?: unknown }).window = undefined;
  });

  test("update forwards id + partial untouched", async () => {
    await menu.update("file.save", { label: "Save As...", enabled: false });
    expect(osCalls).toEqual([
      {
        api: "menu",
        method: "update",
        args: { id: "file.save", partial: { label: "Save As...", enabled: false } },
      },
    ]);
  });

  test("update accepts an empty partial (no-op round trip)", async () => {
    await menu.update("file.save", {});
    expect(osCalls).toEqual([
      { api: "menu", method: "update", args: { id: "file.save", partial: {} } },
    ]);
  });

  test("setItemEnabled forwards id + enabled", async () => {
    await menu.setItemEnabled("save", false);
    expect(osCalls).toEqual([
      { api: "menu", method: "setItemEnabled", args: { id: "save", enabled: false } },
    ]);
  });

  test("setItemLabel forwards id + label", async () => {
    await menu.setItemLabel("user", "Alice");
    expect(osCalls).toEqual([
      { api: "menu", method: "setItemLabel", args: { id: "user", label: "Alice" } },
    ]);
  });

  test("setItemChecked forwards id + checked", async () => {
    await menu.setItemChecked("muted", true);
    expect(osCalls).toEqual([
      { api: "menu", method: "setItemChecked", args: { id: "muted", checked: true } },
    ]);
  });

  test("setItemVisible forwards id + visible", async () => {
    await menu.setItemVisible("debug", false);
    expect(osCalls).toEqual([
      { api: "menu", method: "setItemVisible", args: { id: "debug", visible: false } },
    ]);
  });

  test("convenience wrappers each route through the menu API", async () => {
    await menu.setItemEnabled("x", true);
    await menu.setItemLabel("x", "X");
    await menu.setItemChecked("x", false);
    await menu.setItemVisible("x", true);
    expect(osCalls.map((c) => `${c.api}.${c.method}`)).toEqual([
      "menu.setItemEnabled",
      "menu.setItemLabel",
      "menu.setItemChecked",
      "menu.setItemVisible",
    ]);
  });
});
