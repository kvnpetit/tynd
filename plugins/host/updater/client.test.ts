import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { installTyndMock, type MockOsCall, type TyndMock } from "@tynd/core/testing";
import { createUpdater } from "./client";
import { assertValidChannel, DEFAULT_UPDATER_CHANNEL } from "./updater-config";
import type { UpdaterRuntimeConfig } from "./updater-types";

const base: UpdaterRuntimeConfig = {
  endpoints: ["https://a.example/manifest.json"],
  publicKeys: ["KEY1"],
  preferPatch: true,
  allowPatch: true,
  maxPatchSize: null,
  fallbackToFull: true,
  autoCheck: false,
  checkIntervalSecs: 3600,
  checkOnStartup: false,
  autoDownload: false,
  autoApply: false,
  promptStyle: "modal",
  requireSignature: true,
  requireMajorPrompt: true,
};

describe("assertValidChannel", () => {
  test("accepts url-safe names", () => {
    for (const name of [DEFAULT_UPDATER_CHANNEL, "beta", "nightly-2.1_x"]) {
      expect(() => assertValidChannel(name)).not.toThrow();
    }
  });

  test("rejects empty, path-like and oversized names", () => {
    for (const name of ["", ".x", "a/b", "a b", "a?b", "x".repeat(65)]) {
      expect(() => assertValidChannel(name)).toThrow();
    }
  });
});

const g = globalThis as Record<string, unknown>;

/** Mock bridge per test; `methods()` lists the OS methods called, `app.getVersion` aside. */
function useBridge(options: { windowLabel?: string } = {}) {
  const state = {
    bridge: undefined as unknown as TyndMock,
    checkResult: { available: false } as unknown,
  };

  beforeEach(() => {
    state.checkResult = { available: false };
    state.bridge = installTyndMock({
      ...options,
      os: {
        "updater.check": () => state.checkResult,
        "updater.getChannel": (args) => ({
          channel: (args as { default?: string }).default ?? "stable",
        }),
        "updater.startPeriodicCheck": () => ({ id: 7 }),
        "updater.downloadAndVerify": () => ({ handle: "h", size: 1 }),
      },
      osFallback: () => null,
    });
    g.TYND_VERSION = "1.0.0";
  });

  afterEach(() => {
    state.bridge.restore();
    Reflect.deleteProperty(g, "__TYND_UPDATER_CONFIG__");
    Reflect.deleteProperty(g, "TYND_VERSION");
  });

  const osCalls = () => state.bridge.calls.filter((c): c is MockOsCall => c.kind === "os");
  return {
    state,
    osCalls,
    call: (method: string) => osCalls().find((c) => c.api === "updater" && c.method === method),
    methods: () =>
      osCalls()
        .filter((c) => !(c.api === "app" && c.method === "getVersion"))
        .map((c) => c.method),
    settle: () => new Promise((resolve) => setTimeout(resolve, 10)),
    setCheckResult: (v: unknown) => {
      state.checkResult = v;
    },
  };
}

describe("updater facade: check", () => {
  const host = useBridge();

  test("checkForUpdates sends no endpoints, channel, version or keys: the host uses its trusted config", async () => {
    g.__TYND_UPDATER_CONFIG__ = {
      ...base,
      endpoints: ["https://m1/{{channel}}/m.json", "https://m2/{{channel}}/m.json"],
      publicKeys: ["KEY2", "KEY1"],
      channel: "beta",
    };
    const { updater } = await import("./client");
    const info = await updater.checkForUpdates();
    expect(info.available).toBe(false);
    expect(host.call("check")?.args).toEqual({});
  });
});

describe("updater facade: channels", () => {
  const host = useBridge();

  test("setChannel validates locally, then persists through the host", async () => {
    g.__TYND_UPDATER_CONFIG__ = base;
    const { updater } = await import("./client");
    await expect(updater.setChannel("a/b")).rejects.toThrow();
    expect(host.call("setChannel")).toBeUndefined();

    await updater.setChannel("beta");
    expect(host.call("setChannel")?.args).toEqual({ channel: "beta" });
  });

  test("getChannel falls back to the configured default", async () => {
    g.__TYND_UPDATER_CONFIG__ = { ...base, channel: "beta" };
    const { updater } = await import("./client");
    expect(await updater.getChannel()).toBe("beta");
    expect(host.call("getChannel")?.args).toEqual({ default: "beta" });
  });
});

const available = {
  available: true,
  version: "1.1.0",
  url: "https://a.example/app",
  signature: "sig",
  size: 10,
  notes: null,
  pubDate: null,
  patch: null,
};

describe("updater facade: startup and periodic checks", () => {
  const { call, methods, settle } = useBridge();

  test("nothing runs when autoCheck and checkOnStartup are off", async () => {
    g.__TYND_UPDATER_CONFIG__ = base;
    createUpdater();
    await settle();
    expect(methods()).toEqual([]);
  });

  test("checkOnStartup alone runs one check at startup", async () => {
    g.__TYND_UPDATER_CONFIG__ = { ...base, checkOnStartup: true };
    createUpdater();
    await settle();
    expect(methods()).toEqual(["check"]);
  });

  test("autoCheck + checkOnStartup starts the host periodic check with the interval", async () => {
    g.__TYND_UPDATER_CONFIG__ = {
      ...base,
      autoCheck: true,
      checkOnStartup: true,
      checkIntervalSecs: 21600,
    };
    createUpdater();
    await settle();
    expect(call("startPeriodicCheck")?.args).toEqual({ intervalMs: 21_600_000 });
    expect(methods()).not.toContain("check");
  });

  test("autoCheck without checkOnStartup waits one interval before the first check", async () => {
    g.__TYND_UPDATER_CONFIG__ = { ...base, autoCheck: true, checkIntervalSecs: 3600 };
    const updater = createUpdater();
    await settle();
    expect(methods()).not.toContain("startPeriodicCheck");
    await updater.cancel();
  });
});

describe("updater facade: secondary windows", () => {
  const { methods, settle } = useBridge({ windowLabel: "settings" });

  test("secondary windows do not run the automation", async () => {
    g.__TYND_UPDATER_CONFIG__ = { ...base, checkOnStartup: true };
    createUpdater();
    await settle();
    expect(methods()).toEqual([]);
  });
});

describe("updater facade: downloads and failures", () => {
  const { state, methods, settle, setCheckResult } = useBridge();

  test("periodic results update state and autoDownload chains into download()", async () => {
    g.__TYND_UPDATER_CONFIG__ = {
      ...base,
      autoCheck: true,
      checkOnStartup: true,
      autoDownload: true,
      requireMajorPrompt: false,
    };
    const updater = createUpdater();
    const found: unknown[] = [];
    updater.on("update-available", (info) => found.push(info));
    await settle();
    state.bridge.emitOs("updater:check", { id: 7, ok: true, info: available });
    await settle();
    expect(found).toHaveLength(1);
    expect(await updater.getAvailableVersion()).toBe("1.1.0");
    expect(methods()).toContain("downloadAndVerify");
  });

  test("autoDownload is skipped for a major bump while requireMajorPrompt is set", async () => {
    g.__TYND_UPDATER_CONFIG__ = {
      ...base,
      checkOnStartup: true,
      autoDownload: true,
      requireMajorPrompt: true,
    };
    setCheckResult({ ...available, version: "2.0.0" });
    createUpdater();
    await settle();
    expect(methods()).toEqual(["check"]);
  });

  test("a failed route choice clears the stale pending update and emits error", async () => {
    g.__TYND_UPDATER_CONFIG__ = base;
    setCheckResult(available);
    const updater = createUpdater();
    const errors: Error[] = [];
    updater.on("error", (e) => errors.push(e));
    await updater.checkForUpdates();
    expect(await updater.getAvailableVersion()).toBe("1.1.0");
    g.__TYND_UPDATER_CONFIG__ = { ...base, fallbackToFull: false };
    // fallbackToFull is off and no patch exists: the check must fail loudly.
    await expect(updater.checkForUpdates()).rejects.toThrow(/fallbackToFull/);
    expect(errors).toHaveLength(1);
    expect(await updater.getAvailableVersion()).toBeNull();
    await expect(updater.download()).rejects.toThrow(/no update info/);
  });
});

describe("updater facade: version", () => {
  let bridge: TyndMock | undefined;
  const hostVersion = (version: unknown) => {
    bridge = installTyndMock({ os: { "app.getVersion": () => version }, osFallback: () => null });
  };
  afterEach(() => {
    bridge?.restore();
    bridge = undefined;
    Reflect.deleteProperty(g, "TYND_VERSION");
  });

  test("getCurrentVersion asks the host instead of falling back to 0.0.0", async () => {
    hostVersion("3.2.1");
    expect(await createUpdater().getCurrentVersion()).toBe("3.2.1");
  });

  test("host-reported version wins over a renderer override", async () => {
    g.TYND_VERSION = "9.9.9";
    hostVersion("3.2.1");
    expect(await createUpdater().getCurrentVersion()).toBe("3.2.1");
  });

  test("falls back to local detection when the host has no version", async () => {
    g.TYND_VERSION = "1.0.0";
    hostVersion("0.0.0");
    expect(await createUpdater().getCurrentVersion()).toBe("1.0.0");
  });
});
