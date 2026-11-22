import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { chmodSync, mkdtempSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  type DiscoveryEntry,
  defaultDiscoveryDir,
  parseDiscovery,
  readDiscoveryEntries,
  selectEntry,
} from "./discovery.ts";

const body = (overrides: Record<string, unknown> = {}) =>
  JSON.stringify({
    pid: 4242,
    appId: "dev.example.app",
    appVersion: "1.0.0",
    endpoint: { unix: "/tmp/tynd-app-100.sock" },
    startedAt: "2026-11-07T10:00:00Z",
    cookie: "abc",
    ...overrides,
  });

describe("parseDiscovery", () => {
  test("reads a unix endpoint", () => {
    expect(parseDiscovery(body(), 5)).toEqual({
      pid: 4242,
      appId: "dev.example.app",
      endpoint: { unix: "/tmp/tynd-app-100.sock" },
      token: "abc",
      startedAt: "2026-11-07T10:00:00Z",
      mtimeMs: 5,
    });
  });

  test("reads a tcp endpoint", () => {
    const entry = parseDiscovery(body({ endpoint: { tcp: 9000 } }));
    expect(entry?.endpoint).toEqual({ tcp: 9000, host: undefined });
  });

  test("reads the token from the cookie field and rejects entries without one", () => {
    expect(parseDiscovery(body({ cookie: "c" }))?.token).toBe("c");
    expect(parseDiscovery(body({ cookie: undefined }))).toBeUndefined();
    expect(parseDiscovery(body({ cookie: undefined, token: "t" }))).toBeUndefined();
  });

  test("rejects malformed input", () => {
    expect(parseDiscovery("not json")).toBeUndefined();
    expect(parseDiscovery(body({ endpoint: {} }))).toBeUndefined();
    expect(parseDiscovery(body({ pid: "1" }))).toBeUndefined();
  });
});

describe("readDiscoveryEntries", () => {
  let dir: string;
  beforeEach(() => {
    dir = mkdtempSync(join(tmpdir(), "tynd-test-disc-"));
  });
  afterEach(() => {
    rmSync(dir, { recursive: true, force: true });
  });

  test("reads valid files and skips junk", () => {
    writeFileSync(join(dir, "1.json"), body({ pid: 1 }));
    writeFileSync(join(dir, "2.json"), "garbage");
    writeFileSync(join(dir, "notes.txt"), body({ pid: 3 }));
    expect(readDiscoveryEntries(dir).map((entry) => entry.pid)).toEqual([1]);
  });

  test("a missing directory is empty", () => {
    expect(readDiscoveryEntries(join(dir, "absent"))).toEqual([]);
  });
});

describe("selectEntry", () => {
  const entry = (pid: number, unix: string, mtimeMs: number, appId = "app"): DiscoveryEntry => ({
    pid,
    appId,
    endpoint: { unix },
    token: "t",
    startedAt: "",
    mtimeMs,
  });
  const alive = () => true;

  test("ignores entries older than the launch and dead processes", () => {
    const entries = [entry(1, "/a-1.sock", 50), entry(2, "/a-2.sock", 200)];
    expect(selectEntry(entries, { notBefore: 100, isAlive: alive })?.pid).toBe(2);
    expect(selectEntry(entries, { notBefore: 100, isAlive: () => false })).toBeUndefined();
  });

  test("prefers the exact requested socket", () => {
    const entries = [entry(1, "/x/other.sock", 300), entry(2, "/x/mine.sock", 200)];
    expect(
      selectEntry(entries, { notBefore: 0, socket: "/x/mine.sock", isAlive: alive })?.pid,
    ).toBe(2);
  });

  test("never falls back to another entry when an exact socket was requested", () => {
    const entries = [
      entry(1, "/x/other.sock", 300),
      entry(2, "/tmp/tynd-app-777.sock", 200),
      { ...entry(3, "", 100), endpoint: { tcp: 9000 } },
    ];
    expect(
      selectEntry(entries, {
        notBefore: 0,
        socket: "/x/mine.sock",
        launcherPid: 777,
        isAlive: alive,
      }),
    ).toBeUndefined();
  });

  test("then the socket named after the launcher pid (tynd dev)", () => {
    const entries = [
      entry(1, "/tmp/tynd-app-999.sock", 300),
      entry(2, "/tmp/tynd-app-777.sock", 200),
      entry(3, "\\\\.\\pipe\\tynd-app-777", 100),
    ];
    expect(selectEntry(entries, { notBefore: 0, launcherPid: 777, isAlive: alive })?.pid).toBe(2);
    expect(
      selectEntry(entries.slice(2), { notBefore: 0, launcherPid: 777, isAlive: alive })?.pid,
    ).toBe(3);
  });

  test("falls back to the newest entry and honors the app id filter", () => {
    const entries = [entry(1, "/a.sock", 100, "one"), entry(2, "/b.sock", 300, "two")];
    expect(selectEntry(entries, { notBefore: 0, isAlive: alive })?.pid).toBe(2);
    expect(selectEntry(entries, { notBefore: 0, appId: "one", isAlive: alive })?.pid).toBe(1);
    expect(selectEntry(entries, { notBefore: 0, appId: "zzz", isAlive: alive })).toBeUndefined();
  });
});

describe("defaultDiscoveryDir", () => {
  test("follows the host platform conventions", () => {
    expect(defaultDiscoveryDir({ LOCALAPPDATA: "C:\\L" }, "win32")).toBe(
      join("C:\\L", "tynd", "debug"),
    );
    expect(defaultDiscoveryDir({ XDG_RUNTIME_DIR: "/run/u" }, "linux")).toBe(
      join("/run/u", "tynd"),
    );
    expect(defaultDiscoveryDir({ TYND_TEST_DISCOVERY_DIR: "/custom" }, "linux")).toBe("/custom");
  });

  test("on Linux without XDG_RUNTIME_DIR uses /tmp/tynd-<euid>, ignoring TMPDIR", () => {
    expect(defaultDiscoveryDir({ TMPDIR: "/var/tmp/x" }, "linux", 1000)).toBe(
      join("/tmp", "tynd-1000"),
    );
  });
});

describe.skipIf(process.platform === "win32")("readDiscoveryEntries ownership checks", () => {
  let dir: string;
  beforeEach(() => {
    dir = mkdtempSync(join(tmpdir(), "tynd-test-own-"));
    writeFileSync(join(dir, "1.json"), body({ pid: 1 }));
  });
  afterEach(() => {
    rmSync(dir, { recursive: true, force: true });
  });

  test("accepts a 0700 directory owned by the current user", () => {
    chmodSync(dir, 0o700);
    expect(readDiscoveryEntries(dir).map((entry) => entry.pid)).toEqual([1]);
  });

  test("skips a directory that is not mode 0700", () => {
    chmodSync(dir, 0o755);
    expect(readDiscoveryEntries(dir)).toEqual([]);
  });

  test("skips a directory or files owned by another uid", () => {
    const other = (process.geteuid?.() ?? 0) + 1;
    expect(readDiscoveryEntries(dir, other)).toEqual([]);
  });

  test("skips a symlinked directory", () => {
    const link = `${dir}-link`;
    symlinkSync(dir, link);
    try {
      expect(readDiscoveryEntries(link)).toEqual([]);
    } finally {
      rmSync(link, { force: true });
    }
  });

  test("skips a symlinked file", () => {
    const target = join(tmpdir(), `tynd-test-target-${process.pid}.json`);
    writeFileSync(target, body({ pid: 2 }));
    symlinkSync(target, join(dir, "2.json"));
    try {
      expect(readDiscoveryEntries(dir).map((entry) => entry.pid)).toEqual([1]);
    } finally {
      rmSync(target, { force: true });
    }
  });
});
