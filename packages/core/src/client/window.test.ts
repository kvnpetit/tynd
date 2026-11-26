import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { installTyndMock, type MockOsCall, type TyndMock } from "../testing";
import { tyndWindow } from "./window";

let bridge: TyndMock;
beforeEach(() => {
  bridge = installTyndMock({ osFallback: () => null });
});
afterEach(() => bridge.restore());

const osCalls = () => bridge.calls.filter((call): call is MockOsCall => call.kind === "os");
const lastCall = () => osCalls().at(-1);
const sent = () => osCalls().map(({ method, args }) => ({ method, args }));

describe("tyndWindow frame controls", () => {
  test("setAlwaysOnTop sends the level only when one is given", async () => {
    await tyndWindow.setAlwaysOnTop(true);
    expect(lastCall()).toEqual({
      kind: "os",
      api: "window",
      method: "setAlwaysOnTop",
      args: { always: true },
    });
    await tyndWindow.setAlwaysOnTop(true, { level: "screen-saver" });
    expect(lastCall()?.args).toEqual({ always: true, level: "screen-saver" });
  });

  test("always-on-bottom, kiosk and simple fullscreen forward their flag", async () => {
    await tyndWindow.setAlwaysOnBottom(true);
    await tyndWindow.setKiosk(true);
    await tyndWindow.setSimpleFullscreen(false);
    expect(sent()).toEqual([
      { method: "setAlwaysOnBottom", args: { bottom: true } },
      { method: "setKiosk", args: { enabled: true } },
      { method: "setSimpleFullscreen", args: { fullscreen: false } },
    ]);
  });

  test("opacity, aspect ratio and movability", async () => {
    await tyndWindow.setOpacity(0.8);
    await tyndWindow.setAspectRatio(16 / 9);
    await tyndWindow.setAspectRatio(null);
    await tyndWindow.setMovable(false);
    expect(sent()).toEqual([
      { method: "setOpacity", args: { opacity: 0.8 } },
      { method: "setAspectRatio", args: { ratio: 16 / 9 } },
      { method: "setAspectRatio", args: { ratio: null } },
      { method: "setMovable", args: { movable: false } },
    ]);
  });
});

describe("tyndWindow attention, cursor and getters", () => {
  test("requestAttention defaults to informational; cancelAttention stops it", async () => {
    await tyndWindow.requestAttention();
    expect(lastCall()?.args).toEqual({ kind: "informational" });
    await tyndWindow.requestAttention("critical");
    expect(lastCall()?.args).toEqual({ kind: "critical" });
    await tyndWindow.cancelAttention();
    expect(lastCall()?.method).toBe("cancelAttention");
  });

  test("cursor grab forwards its flag", async () => {
    await tyndWindow.setCursorGrab(true);
    expect(lastCall()).toEqual({
      kind: "os",
      api: "window",
      method: "setCursorGrab",
      args: { grab: true },
    });
  });

  test("getters resolve the host value", async () => {
    bridge.restore();
    bridge = installTyndMock({
      osFallback: (_api, method) =>
        method === "getInnerPosition" ? { x: 10, y: 40 } : method === "isKiosk",
    });
    expect(await tyndWindow.getInnerPosition()).toEqual({ x: 10, y: 40 });
    expect(await tyndWindow.isKiosk()).toBe(true);
    expect(await tyndWindow.isAlwaysOnTop()).toBe(false);
    expect(await tyndWindow.isMinimizable()).toBe(false);
    expect(await tyndWindow.isMaximizable()).toBe(false);
  });
});
