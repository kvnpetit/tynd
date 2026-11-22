import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { installTyndMock, type MockOsCall, type TyndMock } from "../testing";
import { app } from "./app";

let bridge: TyndMock;
beforeEach(() => {
  bridge = installTyndMock({ osFallback: () => null });
});
afterEach(() => bridge.restore());

const osCalls = () => bridge.calls.filter((call): call is MockOsCall => call.kind === "os");

describe("app recent documents", () => {
  test("addRecentDocument sends the path", async () => {
    await app.addRecentDocument("/docs/report.pdf");
    expect(bridge.calls).toEqual([
      { kind: "os", api: "app", method: "addRecentDocument", args: { path: "/docs/report.pdf" } },
    ]);
  });

  test("clearRecentDocuments takes no arguments", async () => {
    await app.clearRecentDocuments();
    expect(bridge.calls).toHaveLength(1);
    expect(bridge.calls[0]).toMatchObject({
      kind: "os",
      api: "app",
      method: "clearRecentDocuments",
    });
  });
});

describe("app dock", () => {
  test("show / hide map to the activation policy", async () => {
    await app.dock.hide();
    await app.dock.show();
    await app.setActivationPolicy("prohibited");
    expect(osCalls().map((c) => c.args)).toEqual([
      { policy: "accessory" },
      { policy: "regular" },
      { policy: "prohibited" },
    ]);
    expect(osCalls().every((c) => c.method === "setActivationPolicy")).toBe(true);
  });

  test("bounce defaults to informational; setIcon and setMenu forward their arguments", async () => {
    await app.dock.bounce();
    await app.dock.bounce("critical");
    await app.dock.cancelBounce(7);
    await app.dock.setIcon(null);
    await app.dock.setMenu([{ label: "New window", id: "new" }]);
    expect(osCalls().map(({ method, args }) => ({ method, args }))).toEqual([
      { method: "dockBounce", args: { kind: "informational" } },
      { method: "dockBounce", args: { kind: "critical" } },
      { method: "dockCancelBounce", args: { id: 7 } },
      { method: "setDockIcon", args: { path: null } },
      { method: "setDockMenu", args: { items: [{ label: "New window", id: "new" }] } },
    ]);
  });
});

describe("app jump list", () => {
  test("setJumpList forwards tasks, categories and showRecent", async () => {
    const list = {
      tasks: [{ title: "New window", args: ["--new-window"], description: "Open a window" }],
      categories: [
        {
          name: "Projects",
          items: [{ title: "Acme", args: ["--open", "C:/work/acme"], icon: "C:/icons/acme.ico" }],
        },
      ],
      showRecent: false,
    };
    await app.setJumpList(list);
    await app.clearJumpList();
    expect(osCalls().map(({ method, args }) => ({ method, args }))).toEqual([
      { method: "setJumpList", args: list },
      { method: "clearJumpList", args: null },
    ]);
  });
});
