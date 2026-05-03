import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { updater, updaterIpc } from "./updater";

interface OsCall {
  api: string;
  method: string;
  args: unknown;
}

interface ShimOptions {
  onOsCall?: (c: OsCall) => unknown;
  /** Pre-baked updater config for `globalThis.__TYND_UPDATER_CONFIG__`. */
  config?: Record<string, unknown> | null;
  version?: string;
}

interface Shim {
  osCalls: OsCall[];
}

function mount(options: ShimOptions = {}): Shim {
  const osCalls: OsCall[] = [];
  const g = globalThis as unknown as Record<string, unknown>;
  g.window = {
    __tynd__: {
      os_call(api: string, method: string, args: unknown) {
        const call = { api, method, args };
        osCalls.push(call);
        return Promise.resolve(options.onOsCall?.(call) ?? null);
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
  if (options.config !== undefined) {
    g.__TYND_UPDATER_CONFIG__ = options.config as unknown;
  } else {
    delete g.__TYND_UPDATER_CONFIG__;
  }
  if (options.version) {
    g.TYND_VERSION = options.version;
  } else {
    delete g.TYND_VERSION;
  }
  return { osCalls };
}

const baseConfig = {
  endpoint: "https://updates.example.test/manifest.json",
  publicKey: "AA",
  preferPatch: true,
  allowPatch: true,
  maxPatchSize: null,
  fallbackToFull: true,
  autoCheck: true,
  checkIntervalSecs: 43_200,
  checkOnStartup: true,
  autoDownload: true,
  autoApply: false,
  promptStyle: "modal",
  requireSignature: true,
  requireMajorPrompt: true,
};

afterEach(() => {
  const g = globalThis as Record<string, unknown>;
  delete g.window;
  delete g.__TYND_UPDATER_CONFIG__;
  delete g.TYND_VERSION;
});

describe("updater facade — high-level API", () => {
  beforeEach(() => {
    mount({ config: baseConfig, version: "1.0.0" });
    // Reset facade-internal pending state by trying a not-available call
    // — the singleton is module-level, so leftover state from a previous
    // test would otherwise bleed into the next.
  });

  test("exposes the canonical Updater interface methods", () => {
    expect(typeof updater.checkForUpdates).toBe("function");
    expect(typeof updater.download).toBe("function");
    expect(typeof updater.apply).toBe("function");
    expect(typeof updater.cancel).toBe("function");
    expect(typeof updater.getCurrentVersion).toBe("function");
    expect(typeof updater.getAvailableVersion).toBe("function");
    expect(typeof updater.getDownloadProgress).toBe("function");
    expect(typeof updater.getConfig).toBe("function");
    expect(typeof updater.on).toBe("function");
  });

  test("getConfig() returns the build-time inlined block", () => {
    const cfg = updater.getConfig();
    expect(cfg).not.toBeNull();
    expect(cfg?.endpoint).toBe(baseConfig.endpoint);
    expect(cfg?.preferPatch).toBe(true);
  });

  test("checkForUpdates() with no patch in manifest selects the full route", async () => {
    mount({
      config: baseConfig,
      version: "1.0.0",
      onOsCall: (c) =>
        c.method === "check"
          ? {
              available: true,
              version: "1.1.0",
              url: "https://...",
              signature: "SIG",
              platform: "windows-x86_64",
              patch: null,
              size: 47_000_000,
            }
          : null,
    });
    const info = await updater.checkForUpdates();
    expect(info.available).toBe(true);
    expect(info.version).toBe("1.1.0");
    expect(info.downloadType).toBe("full");
    expect(info.fullSize).toBe(47_000_000);
    expect(info.isMajorBump).toBe(false);
  });

  test("checkForUpdates() prefers patch when allowed, available, and within size cap", async () => {
    mount({
      config: baseConfig,
      version: "1.0.0",
      onOsCall: (c) =>
        c.method === "check"
          ? {
              available: true,
              version: "1.1.0",
              url: "https://...",
              signature: "SIG",
              platform: "windows-x86_64",
              patch: {
                from: "1.0.0",
                url: "https://patch...",
                signature: "PSIG",
                size: 215_000,
                compression: "zstd",
              },
              size: 47_000_000,
            }
          : null,
    });
    const info = await updater.checkForUpdates();
    expect(info.downloadType).toBe("patch");
    expect(info.patchSize).toBe(215_000);
    expect(info.fromVersion).toBe("1.0.0");
  });

  test("allowPatch=false forces full route even when manifest carries a patch", async () => {
    mount({
      config: { ...baseConfig, allowPatch: false },
      version: "1.0.0",
      onOsCall: (c) =>
        c.method === "check"
          ? {
              available: true,
              version: "1.1.0",
              url: "https://...",
              signature: "SIG",
              platform: "windows-x86_64",
              patch: { from: "1.0.0", url: "https://...", signature: "PSIG", size: 215_000 },
              size: 47_000_000,
            }
          : null,
    });
    const info = await updater.checkForUpdates();
    expect(info.downloadType).toBe("full");
  });

  test("maxPatchSize cap kicks the route to full when the patch exceeds it", async () => {
    mount({
      config: { ...baseConfig, maxPatchSize: 1_000_000 },
      version: "1.0.0",
      onOsCall: (c) =>
        c.method === "check"
          ? {
              available: true,
              version: "1.1.0",
              url: "https://...",
              signature: "SIG",
              platform: "windows-x86_64",
              patch: { from: "1.0.0", url: "https://...", signature: "PSIG", size: 5_000_000 },
              size: 47_000_000,
            }
          : null,
    });
    const info = await updater.checkForUpdates();
    expect(info.downloadType).toBe("full");
  });

  test("fallbackToFull=false + patch unavailable → checkForUpdates rejects", async () => {
    mount({
      config: { ...baseConfig, fallbackToFull: false },
      version: "1.0.0",
      onOsCall: (c) =>
        c.method === "check"
          ? {
              available: true,
              version: "1.1.0",
              url: "https://...",
              signature: "SIG",
              platform: "windows-x86_64",
              patch: null,
              size: 47_000_000,
            }
          : null,
    });
    await expect(updater.checkForUpdates()).rejects.toThrow(/no patch available/);
  });

  test("isMajorBump fires on cross-major version jumps", async () => {
    mount({
      config: baseConfig,
      version: "1.5.7",
      onOsCall: (c) =>
        c.method === "check"
          ? {
              available: true,
              version: "2.0.0",
              url: "https://...",
              signature: "SIG",
              platform: "windows-x86_64",
              patch: null,
              size: 47_000_000,
            }
          : null,
    });
    const info = await updater.checkForUpdates();
    expect(info.isMajorBump).toBe(true);
  });

  test("checkForUpdates() with no manifest update emits update-not-available", async () => {
    mount({
      config: baseConfig,
      version: "1.0.0",
      onOsCall: (c) => (c.method === "check" ? { available: false } : null),
    });
    let notAvailableFired = false;
    const off = updater.on("update-not-available", () => {
      notAvailableFired = true;
    });
    try {
      const info = await updater.checkForUpdates();
      expect(info.available).toBe(false);
      expect(notAvailableFired).toBe(true);
    } finally {
      off();
    }
  });

  test("missing __TYND_UPDATER_CONFIG__ → checkForUpdates throws and emits error", async () => {
    mount({ config: null, version: "1.0.0" });
    let captured: Error | null = null;
    const off = updater.on("error", (e) => {
      captured = e;
    });
    try {
      await expect(updater.checkForUpdates()).rejects.toThrow(/no updater block/);
      expect(captured).not.toBeNull();
    } finally {
      off();
    }
  });

  test("download() without prior checkForUpdates() rejects", async () => {
    // Re-mount with a fresh `available: false` so any prior `#pending`
    // is cleared by the not-available branch.
    mount({
      config: baseConfig,
      version: "1.0.0",
      onOsCall: (c) => (c.method === "check" ? { available: false } : null),
    });
    await updater.checkForUpdates();
    await expect(updater.download()).rejects.toThrow(/no update info/);
  });
});

describe("updaterIpc — low-level dispatch shims", () => {
  test("check() returns null on `available: false`", async () => {
    mount({
      config: baseConfig,
      version: "1.0.0",
      onOsCall: (c) => (c.method === "check" ? { available: false } : null),
    });
    const result = await updaterIpc.check({
      endpoint: "https://updates.example.test/manifest.json",
      currentVersion: "1.0.0",
    });
    expect(result).toBeNull();
  });

  test("install() forwards path + hash + relaunch to the host", async () => {
    const shim = mount({
      config: baseConfig,
      version: "1.0.0",
      onOsCall: (c) =>
        c.method === "install"
          ? { installed: true, path: "C:/path/to/app.exe", relaunch: true }
          : null,
    });
    const result = await updaterIpc.install({
      path: "C:/path/to/app.exe",
      hash: "deadbeef",
      relaunch: true,
    });
    expect(result.installed).toBe(true);
    expect(shim.osCalls.find((c) => c.method === "install")?.args).toMatchObject({
      path: "C:/path/to/app.exe",
      hash: "deadbeef",
      relaunch: true,
    });
  });
});
