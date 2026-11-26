import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { installTyndMock, type TyndMock } from "@tynd/core/testing";
import { keys } from "./client";

let bridge: TyndMock;
beforeEach(() => {
  bridge = installTyndMock({ osFallback: () => null });
});
afterEach(() => bridge.restore());

describe("@tynd/plugin-keys client", () => {
  test("disable", async () => {
    await keys.disable(["Ctrl+R"]);
    expect(bridge.calls).toEqual([
      { kind: "os", api: "keys", method: "disable", args: { combos: ["Ctrl+R"] } },
    ]);
  });

  test("clear", async () => {
    await keys.clear();
    expect(bridge.calls).toEqual([{ kind: "os", api: "keys", method: "clear", args: {} }]);
  });
});

// Tests below drive the client through the full core mock (os_on events, host errors).
const g = globalThis as Record<string, unknown>;
let mock: TyndMock | undefined;
const settle = (ms = 10) => new Promise((resolve) => setTimeout(resolve, ms));

function fakeWindow() {
  const listeners: string[] = [];
  const win = g.window as Record<string, unknown>;
  win.addEventListener = (type: string) => listeners.push(type);
  return listeners;
}

describe("keys.install", () => {
  afterEach(() => {
    mock?.restore();
    mock = undefined;
  });

  test("is idempotent across calls", async () => {
    mock = installTyndMock({ os: { "keys.list": () => [] } });
    const listeners = fakeWindow();
    let osOn = 0;
    const bridge = (g.window as { __tynd__: { os_on: (...a: unknown[]) => unknown } }).__tynd__;
    const original = bridge.os_on.bind(bridge);
    bridge.os_on = (...args: unknown[]) => {
      osOn++;
      return original(...args);
    };
    keys.install();
    keys.install();
    await settle();
    expect(listeners).toEqual(["keydown"]);
    expect(osOn).toBe(1);
  });

  test("a keys:set event wins over a snapshot that resolves later", async () => {
    let resolveList: (v: string[]) => void = () => undefined;
    mock = installTyndMock({
      os: { "keys.list": () => new Promise<string[]>((r) => (resolveList = r)) },
    });
    fakeWindow();
    keys.install();
    mock.emitOs("keys:set", { combos: ["Ctrl+R"] });
    resolveList(["F5"]);
    await settle();
    const set = (g.window as { __tynd_keys_disabled__: Set<string> }).__tynd_keys_disabled__;
    expect([...set]).toEqual(["Ctrl+R"]);
  });

  test("a failing initial snapshot does not raise an unhandled rejection", async () => {
    const unhandled: unknown[] = [];
    const onUnhandled = (e: unknown) => unhandled.push(e);
    process.on("unhandledRejection", onUnhandled);
    try {
      mock = installTyndMock({
        os: {
          "keys.list": () => {
            throw new Error("keys disabled");
          },
        },
      });
      fakeWindow();
      keys.install();
      await settle(20);
      expect(unhandled).toEqual([]);
    } finally {
      process.off("unhandledRejection", onUnhandled);
    }
  });
});
