import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { installTyndMock, type TyndMock } from "@tynd/core/testing";
import { theme } from "./client";

let bridge: TyndMock;
beforeEach(() => {
  bridge = installTyndMock({ osFallback: () => null });
});
afterEach(() => bridge.restore());

describe("@tynd/plugin-theme client", () => {
  test("toggle flips the effective theme and notifies subscribers", () => {
    const seen: string[] = [];
    const off = theme.onChange((next) => seen.push(next));
    const before = theme.current();
    theme.toggle();
    expect(theme.current()).toBe(before === "dark" ? "light" : "dark");
    expect(seen).toEqual([theme.current()]);
    off();
    expect(bridge.calls).toEqual([]);
  });
});

// Tests below drive the client through the full core mock (os_on events, host errors).
let mock: TyndMock | undefined;
const settle = (ms = 10) => new Promise((resolve) => setTimeout(resolve, ms));

describe("theme watcher lifetime", () => {
  afterEach(() => {
    mock?.restore();
    mock = undefined;
  });

  test("dispose without configure does not stop the host watcher; concurrent configure subscribes once", async () => {
    const seen: string[] = [];
    mock = installTyndMock({
      os: {
        "theme.current": () => "dark",
        "theme.startWatch": () => {
          seen.push("start");
        },
        "theme.stopWatch": () => {
          seen.push("stop");
        },
      },
    });
    theme.dispose();
    await settle();
    expect(seen).toEqual([]);

    await Promise.all([
      theme.configure({ htmlClassName: null }),
      theme.configure({ htmlClassName: null }),
    ]);
    expect(seen).toEqual(["start"]);

    const changes: string[] = [];
    theme.onChange((t) => changes.push(t));
    mock.emitOs("theme:change", { mode: "light" });
    mock.emitOs("theme:change", { mode: "dark" });
    // One subscription: each real change notifies once (not twice).
    expect(changes).toEqual(["light", "dark"]);

    theme.dispose();
    await settle();
    expect(seen).toEqual(["start", "stop"]);
  });
});

describe("system preferences", () => {
  afterEach(() => {
    mock?.restore();
    mock = undefined;
  });

  test("systemPreferences reads theme.preferences", async () => {
    const prefs = {
      mode: "dark",
      accentColor: "#0078d4",
      highContrast: false,
      reducedMotion: true,
      reducedTransparency: null,
    };
    mock = installTyndMock({ os: { "theme.preferences": () => prefs } });
    expect(await theme.systemPreferences()).toEqual(prefs as never);
  });

  test("onSystemPreferencesChange holds one watcher reference until dispose", async () => {
    const calls: string[] = [];
    mock = installTyndMock({
      os: {
        "theme.startWatch": () => {
          calls.push("start");
          return { watching: true };
        },
        "theme.stopWatch": () => {
          calls.push("stop");
        },
      },
    });
    const seen: (string | null)[] = [];
    const sub = await theme.onSystemPreferencesChange((p) => seen.push(p.accentColor));
    mock.emitOs("theme:preferences-change", { accentColor: "#ff0000" });
    sub.dispose();
    sub.dispose();
    mock.emitOs("theme:preferences-change", { accentColor: "#00ff00" });
    await settle();
    expect(seen).toEqual(["#ff0000"]);
    expect(calls).toEqual(["start", "stop"]);
  });

  test("onSystemPreferencesChange rejects when the watcher cannot start", async () => {
    mock = installTyndMock({
      os: {
        "theme.startWatch": () => {
          throw new Error("host is shutting down");
        },
      },
    });
    await expect(theme.onSystemPreferencesChange(() => undefined)).rejects.toThrow("shutting down");
  });
});
