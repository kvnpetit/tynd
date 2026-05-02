import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import type { Monitor } from "./monitor";
import { tyndWindow } from "./window";

type OsCall = { api: string; method: string; args: unknown };

interface ShimOptions {
  monitor: Monitor | null;
  outerSize: { width: number; height: number };
}

function mountShim(opts: ShimOptions): { osCalls: OsCall[] } {
  const osCalls: OsCall[] = [];
  (globalThis as unknown as { window: unknown }).window = {
    __tynd__: {
      os_call(api: string, method: string, args: unknown) {
        osCalls.push({ api, method, args });
        if (api === "window" && method === "currentMonitor") {
          return Promise.resolve(opts.monitor);
        }
        if (api === "window" && method === "getOuterSize") {
          return Promise.resolve(opts.outerSize);
        }
        if (api === "window" && method === "setPosition") {
          return Promise.resolve(undefined);
        }
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

afterEach(() => {
  // Avoid leaking the shim between suites.
  (globalThis as { window?: unknown }).window = undefined;
});

function findSetPosition(calls: OsCall[]): { x: number; y: number } | null {
  for (let i = calls.length - 1; i >= 0; i--) {
    const c = calls[i];
    if (c?.api === "window" && c.method === "setPosition") {
      return c.args as { x: number; y: number };
    }
  }
  return null;
}

describe("tyndWindow.positionAt — single monitor, scale 1", () => {
  let osCalls: OsCall[];
  beforeEach(() => {
    ({ osCalls } = mountShim({
      monitor: {
        name: "primary",
        position: { x: 0, y: 0 },
        size: { width: 1920, height: 1080 },
        scale: 1,
        isPrimary: true,
      },
      outerSize: { width: 400, height: 300 },
    }));
  });

  test("topRight uses width/height with padding", async () => {
    await tyndWindow.positionAt("topRight", 20);
    expect(findSetPosition(osCalls)).toEqual({ x: 1920 - 400 - 20, y: 20 });
  });

  test("center is exact monitor middle", async () => {
    await tyndWindow.positionAt("center");
    expect(findSetPosition(osCalls)).toEqual({
      x: (1920 - 400) / 2,
      y: (1080 - 300) / 2,
    });
  });

  test("trayLeft anchors x=0 with taskbar inset", async () => {
    await tyndWindow.positionAt("trayLeft");
    const pos = findSetPosition(osCalls);
    expect(pos?.x).toBe(0);
    // Heuristic taskbar height (Win 40, macOS 25, Linux 32) — just assert > 0.
    expect(pos?.y).toBeGreaterThan(0);
  });

  test("trayRight anchors to the right edge", async () => {
    await tyndWindow.positionAt("trayRight");
    const pos = findSetPosition(osCalls);
    expect(pos?.x).toBe(1920 - 400);
    expect(pos?.y).toBeGreaterThan(0);
  });

  test("trayCenter centers horizontally below the taskbar", async () => {
    await tyndWindow.positionAt("trayCenter");
    const pos = findSetPosition(osCalls);
    expect(pos?.x).toBe((1920 - 400) / 2);
    expect(pos?.y).toBeGreaterThan(0);
  });

  test("trayBottomCenter centers horizontally above the bottom taskbar", async () => {
    await tyndWindow.positionAt("trayBottomCenter");
    const pos = findSetPosition(osCalls);
    expect(pos?.x).toBe((1920 - 400) / 2);
    // Y is monitor height minus window minus taskbar inset.
    expect(pos?.y).toBeLessThan(1080 - 300);
    expect(pos?.y).toBeGreaterThan(0);
  });
});

describe("tyndWindow.positionAt — secondary monitor at offset", () => {
  let osCalls: OsCall[];
  beforeEach(() => {
    // Secondary monitor positioned to the right of the primary, scale 1.
    ({ osCalls } = mountShim({
      monitor: {
        name: "secondary",
        position: { x: 1920, y: 0 },
        size: { width: 2560, height: 1440 },
        scale: 1,
        isPrimary: false,
      },
      outerSize: { width: 400, height: 300 },
    }));
  });

  test("trayLeft on secondary monitor uses monitor origin, not 0", async () => {
    await tyndWindow.positionAt("trayLeft");
    const pos = findSetPosition(osCalls);
    expect(pos?.x).toBe(1920);
  });

  test("trayRight on secondary monitor uses far-right edge", async () => {
    await tyndWindow.positionAt("trayRight");
    const pos = findSetPosition(osCalls);
    expect(pos?.x).toBe(1920 + 2560 - 400);
  });
});

describe("tyndWindow.positionAt — high-DPI (scale 2x)", () => {
  let osCalls: OsCall[];
  beforeEach(() => {
    // 4K monitor reported at physical pixels with scale 2x —
    // logical size is 1920x1080. Position is also physical.
    ({ osCalls } = mountShim({
      monitor: {
        name: "retina",
        position: { x: 0, y: 0 },
        size: { width: 3840, height: 2160 },
        scale: 2,
        isPrimary: true,
      },
      outerSize: { width: 400, height: 300 },
    }));
  });

  test("trayCenter uses logical pixels (physical / scale)", async () => {
    await tyndWindow.positionAt("trayCenter");
    const pos = findSetPosition(osCalls);
    // Expected x in logical pixels: (1920 - 400) / 2.
    expect(pos?.x).toBe((1920 - 400) / 2);
  });

  test("trayBottomCenter Y uses logical monitor height", async () => {
    await tyndWindow.positionAt("trayBottomCenter");
    const pos = findSetPosition(osCalls);
    expect(pos?.x).toBe((1920 - 400) / 2);
    // Y must be < logical bottom of window, > 0.
    expect(pos?.y).toBeLessThan(1080 - 300);
    expect(pos?.y).toBeGreaterThan(0);
  });
});

describe("tyndWindow.positionAt — null monitor short-circuits", () => {
  test("does not attempt setPosition when currentMonitor is null", async () => {
    const { osCalls } = mountShim({
      monitor: null,
      outerSize: { width: 400, height: 300 },
    });
    await tyndWindow.positionAt("topRight");
    expect(findSetPosition(osCalls)).toBeNull();
  });
});
