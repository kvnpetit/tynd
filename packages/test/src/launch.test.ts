import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { isProcessAlive } from "./discovery.ts";
import {
  defaultLaunchTimeout,
  descendantPids,
  launchApp,
  livePids,
  type TyndApp,
} from "./launch.ts";

const fakeHost = join(import.meta.dir, "fixtures", "fake-host.ts");

describe("launchApp", () => {
  let dir: string;
  const apps: TyndApp[] = [];

  beforeEach(() => {
    dir = mkdtempSync(join(tmpdir(), "tynd-test-launch-"));
  });
  afterEach(async () => {
    await Promise.all(apps.splice(0).map((app) => app.close()));
    rmSync(dir, { recursive: true, force: true });
  });

  const launch = async (extra: string[] = [], timeout = 15_000) => {
    const app = await launchApp({
      command: [process.execPath, fakeHost],
      args: extra,
      env: { TYND_TEST_DISCOVERY_DIR: dir },
      discoveryDir: dir,
      timeout,
    });
    apps.push(app);
    return app;
  };

  test("starts the process, finds its discovery file, authenticates and waits for a window", async () => {
    const app = await launch();
    expect(app.entry.appId).toBe("dev.tynd.fake");
    expect(app.pid).toBeGreaterThan(0);
    const windows = await app.windowList();
    expect(windows.map((window) => window.label)).toEqual(["main"]);
    expect(await app.eval<unknown>("document.readyState")).toBe("complete");
  });

  test("close() quits the app and is idempotent", async () => {
    const app = await launch();
    await app.close();
    await app.close();
    expect(app.closed).toBe(true);
    await expect(app.windowList()).rejects.toThrow();
  });

  test("waitFor and host errors go through the live connection", async () => {
    const app = await launch();
    // `.rejects` never settles on a Windows named pipe in Bun's test runner; catch explicitly.
    const failure = await app.call("nope").catch((caught: unknown) => caught);
    expect(String(failure)).toContain("method not found");
    expect(await app.waitFor(async () => (await app.windowList()).length > 0)).toBe(true);
  });

  test("fails with the app output when the process exits before exposing a socket", async () => {
    const error = await launch(["--exit-early"], 10_000).catch((caught: unknown) => caught);
    expect(String(error)).toContain("exited with code 3");
    expect(String(error)).toContain("fake host: boom");
  });

  test("passes the requested debug socket to the app and attaches through it", async () => {
    const app = await launch();
    const endpoint = app.entry.endpoint;
    expect("unix" in endpoint && endpoint.unix).toMatch(/tynd-test-/);
  });

  test("honors a caller-provided TYND_DEBUG_SOCKET", async () => {
    const socket =
      process.platform === "win32"
        ? `\\\\.\\pipe\\tynd-test-custom-${process.pid}`
        : join(dir, "custom.sock");
    const app = await launchApp({
      command: [process.execPath, fakeHost],
      env: { TYND_TEST_DISCOVERY_DIR: dir, TYND_DEBUG_SOCKET: socket },
      discoveryDir: dir,
      timeout: 15_000,
    });
    apps.push(app);
    expect(app.entry.endpoint).toEqual({ unix: socket });
  });

  test("does not attach to a host that ignores the requested socket", async () => {
    const error = await launch(["--ignore-socket"], 1_500).catch((caught: unknown) => caught);
    expect(String(error)).toContain("no debug socket appeared");
  }, 30_000);

  test("keeps the launch timeout below the per-test timeout", () => {
    expect(defaultLaunchTimeout({})).toBe(120_000);
    expect(defaultLaunchTimeout({ TYND_TEST_TIMEOUT_MS: "60000" })).toBe(50_000);
    expect(defaultLaunchTimeout({ TYND_TEST_TIMEOUT_MS: "600000" })).toBe(120_000);
    expect(defaultLaunchTimeout({ TYND_TEST_TIMEOUT_MS: "junk" })).toBe(120_000);
  });

  test.skipIf(process.platform === "win32")(
    "a failed launch kills the whole process tree, not just the launcher",
    async () => {
      const pidFile = join(dir, "grandchild.pid");
      const error = await launchApp({
        // Like `tynd dev`: the launcher has a long-lived child of its own.
        command: [
          "sh",
          "-c",
          'sleep 300 & echo $! > "$0"; exec "$1" "$2" --no-window',
          pidFile,
          process.execPath,
          fakeHost,
        ],
        env: { TYND_TEST_DISCOVERY_DIR: dir },
        discoveryDir: dir,
        timeout: 2_500,
      }).catch((caught: unknown) => caught);
      expect(String(error)).toContain("app window ready");
      const grandchild = Number(readFileSync(pidFile, "utf8").trim());
      expect(grandchild).toBeGreaterThan(0);
      for (let i = 0; i < 20 && isProcessAlive(grandchild); i++) await Bun.sleep(100);
      expect(isProcessAlive(grandchild)).toBe(false);
    },
  );

  test("fails when the window never becomes ready", async () => {
    const error = await launch(["--no-window"], 1_500).catch((caught: unknown) => caught);
    expect(String(error)).toContain("app window ready");
  });
});

describe("livePids", () => {
  // Regression: the launch-failure teardown spawned `taskkill` for an already-exited tree
  // twice (TyndApp.close, then launchApp's catch), ~1.5 s of blocking spawnSync under load.
  test("keeps only valid, running pids so dead trees never spawn taskkill", () => {
    const running = new Set([10, 12]);
    expect(livePids([10, 11, 12, 0, -1, Number.NaN], (pid) => running.has(pid))).toEqual([10, 12]);
    expect(livePids([process.pid])).toEqual([process.pid]);
    expect(livePids([2 ** 31 - 2])).toEqual([]);
  });
});

describe("descendantPids", () => {
  test("collects roots and every transitive child, ignoring unrelated processes", () => {
    const edges: Array<[number, number]> = [
      [10, 1],
      [11, 10],
      [12, 11],
      [20, 2],
      [13, 10],
    ];
    expect(descendantPids([10], edges).sort((a, b) => a - b)).toEqual([10, 11, 12, 13]);
    expect(descendantPids([10, 2], edges).sort((a, b) => a - b)).toEqual([2, 10, 11, 12, 13, 20]);
  });

  test("drops invalid roots", () => {
    expect(descendantPids([Number.NaN, 0, -3], [[5, 0]])).toEqual([]);
  });
});
