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

describe("@tynd/plugin-dialog message boxes", () => {
  test("message forwards buttons and checkbox and returns the host result", async () => {
    bridge.restore();
    bridge = installTyndMock({ osFallback: () => ({ response: 1, checkboxChecked: true }) });
    const result = await dialog.message("Save changes?", {
      buttons: ["Save", "Don't save", "Cancel"],
      defaultId: 0,
      cancelId: 2,
      checkboxLabel: "Remember",
      kind: "question",
    });
    expect(result).toEqual({ response: 1, checkboxChecked: true });
    expect(bridge.calls).toEqual([
      {
        kind: "os",
        api: "dialog",
        method: "message",
        args: {
          message: "Save changes?",
          buttons: ["Save", "Don't save", "Cancel"],
          defaultId: 0,
          cancelId: 2,
          checkboxLabel: "Remember",
          kind: "question",
        },
      },
    ]);
  });
});

describe("@tynd/plugin-dialog pickers", () => {
  test("openDirectories forwards picker options", async () => {
    await dialog.openDirectories({ showHiddenFiles: true, parent: false });
    expect(bridge.calls).toEqual([
      {
        kind: "os",
        api: "dialog",
        method: "openDirectories",
        args: { showHiddenFiles: true, parent: false },
      },
    ]);
  });

  test("saveFile forwards grantAccess", async () => {
    await dialog.saveFile({ defaultName: "report.pdf", grantAccess: true });
    expect(bridge.calls).toEqual([
      {
        kind: "os",
        api: "dialog",
        method: "saveFile",
        args: { defaultName: "report.pdf", grantAccess: true },
      },
    ]);
  });

  test("confirm sends detail and kind", async () => {
    await dialog.confirm("Delete?", { detail: "Cannot be undone", kind: "warning" });
    expect(bridge.calls).toEqual([
      {
        kind: "os",
        api: "dialog",
        method: "confirm",
        args: { message: "Delete?", detail: "Cannot be undone", kind: "warning" },
      },
    ]);
  });
});
