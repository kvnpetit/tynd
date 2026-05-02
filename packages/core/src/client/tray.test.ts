import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { tray } from "./tray";

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

describe("tray dynamic menu API — wire format", () => {
  let osCalls: OsCall[];
  beforeEach(() => {
    ({ osCalls } = mountShim());
  });
  afterEach(() => {
    (globalThis as { window?: unknown }).window = undefined;
  });

  test("setItemEnabled forwards id + enabled", async () => {
    await tray.setItemEnabled("save", false);
    expect(osCalls).toEqual([
      { api: "tray", method: "setItemEnabled", args: { id: "save", enabled: false } },
    ]);
  });

  test("setItemLabel forwards id + label", async () => {
    await tray.setItemLabel("user", "Alice");
    expect(osCalls).toEqual([
      { api: "tray", method: "setItemLabel", args: { id: "user", label: "Alice" } },
    ]);
  });

  test("setItemChecked forwards id + checked", async () => {
    await tray.setItemChecked("muted", true);
    expect(osCalls).toEqual([
      { api: "tray", method: "setItemChecked", args: { id: "muted", checked: true } },
    ]);
  });

  test("setItemVisible forwards id + visible", async () => {
    await tray.setItemVisible("debug", false);
    expect(osCalls).toEqual([
      { api: "tray", method: "setItemVisible", args: { id: "debug", visible: false } },
    ]);
  });

  test("methods chain via osCall returning void", async () => {
    // Collapsed sequence — none of the setters returns a meaningful value;
    // they just acknowledge the round trip. Failing here would indicate the
    // proxy started swallowing or duplicating calls.
    await tray.setItemEnabled("x", true);
    await tray.setItemLabel("x", "X");
    await tray.setItemChecked("x", false);
    await tray.setItemVisible("x", true);
    expect(osCalls.map((c) => c.method)).toEqual([
      "setItemEnabled",
      "setItemLabel",
      "setItemChecked",
      "setItemVisible",
    ]);
  });
});
