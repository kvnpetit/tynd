import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { installTyndMock, type TyndMock } from "@tynd/core/testing";
import { dialog } from "./client";

let bridge: TyndMock;
beforeEach(() => {
  bridge = installTyndMock({ osFallback: () => null });
});
afterEach(() => bridge.restore());

describe("@tynd/plugin-dialog client", () => {
  test("openFile sends null without options", async () => {
    await dialog.openFile();
    expect(bridge.calls).toEqual([{ kind: "os", api: "dialog", method: "openFile", args: null }]);
  });

  test("warn forces the warning kind", async () => {
    await dialog.warn("Careful", { title: "T" });
    expect(bridge.calls).toEqual([
      {
        kind: "os",
        api: "dialog",
        method: "message",
        args: { message: "Careful", title: "T", kind: "warning" },
      },
    ]);
  });
});
