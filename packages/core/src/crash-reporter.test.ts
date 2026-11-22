import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { mkdtemp, readdir, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { crashReporter, onRenderProcessGone, resetCrashReporterForTests } from "./crash-reporter";
import { resolveHostCall, setHostCallTransport } from "./host";
import { dispatchHostEvent } from "./host-events";

interface Frame {
  id: string;
  api: string;
  method: string;
  args: Record<string, unknown>;
}

let dir: string;
let frames: Frame[];

beforeEach(async () => {
  dir = await mkdtemp(join(tmpdir(), "tynd-crash-"));
  frames = [];
  setHostCallTransport((frame) => {
    frames.push(frame as Frame);
    const answer =
      frame.api === "crashReporter" ? { directory: (frame.args as Frame["args"]).directory } : null;
    queueMicrotask(() => resolveHostCall(frame.id, true, answer));
  });
});
afterEach(async () => {
  resetCrashReporterForTests();
  setHostCallTransport(null);
  await rm(dir, { recursive: true, force: true });
});

const report = (kind: string, time: number) =>
  JSON.stringify({
    kind,
    time,
    message: `${kind} at ${time}`,
    appVersion: null,
    os: "x",
    arch: "y",
  });

describe("crashReporter", () => {
  test("start configures the host and hands unseen reports to onCrash once, oldest first", async () => {
    await writeFile(join(dir, "backend-exit-20-1.json"), report("backend-exit", 20));
    await writeFile(join(dir, "host-panic-10-1.json"), report("host-panic", 10));
    await writeFile(join(dir, "notes.txt"), "ignored");
    const seen: number[] = [];
    await crashReporter.start({ directory: dir, onCrash: (r) => seen.push(r.time) });
    expect(frames[0]).toMatchObject({ api: "crashReporter", method: "start" });
    expect(seen).toEqual([10, 20]);

    resetCrashReporterForTests();
    const again: number[] = [];
    await crashReporter.start({ directory: dir, onCrash: (r) => again.push(r.time) });
    expect(again).toEqual([]);
  });

  test("getCrashReports lists newest first and getLastCrash returns the newest", async () => {
    await writeFile(join(dir, "host-panic-10-1.json"), report("host-panic", 10));
    await writeFile(join(dir, "backend-exit-30-1.json"), report("backend-exit", 30));
    await writeFile(join(dir, "backend-exit-40-1.json"), "{ half written");
    await crashReporter.start({ directory: dir });
    const reports = await crashReporter.getCrashReports();
    expect(reports.map((r) => r.time)).toEqual([30, 10]);
    expect(reports[0]?.file).toBe(join(dir, "backend-exit-30-1.json"));
    expect((await crashReporter.getLastCrash())?.kind).toBe("backend-exit");
  });

  test("reading before start fails with what to do", () => {
    expect(() => crashReporter.getCrashReports()).toThrow("crashReporter.start()");
  });

  test("a renderer loss becomes a report and reaches onCrash", async () => {
    const seen: string[] = [];
    await crashReporter.start({
      directory: dir,
      onCrash: (r) => seen.push(`${r.kind}:${r.label}`),
    });
    dispatchHostEvent("webview:process-gone", { label: "main", reason: "crashed" });
    await Bun.sleep(20);
    expect(seen).toEqual(["render-process-gone:main"]);
    const files = await readdir(dir);
    expect(files.some((f) => f.startsWith("render-process-gone-"))).toBe(true);
  });

  test("bad options are rejected", async () => {
    await expect(crashReporter.start({ directory: 1 as unknown as string })).rejects.toThrow(
      "directory",
    );
  });
});

describe("render-process-gone", () => {
  test("event carries label, reason and a reload helper", async () => {
    let reload: (() => Promise<void>) | undefined;
    const off = onRenderProcessGone((event) => {
      expect(event).toMatchObject({ label: "editor", reason: "unresponsive" });
      reload = event.reload;
    });
    dispatchHostEvent("webview:process-gone", { label: "editor", reason: "unresponsive" });
    dispatchHostEvent("webview:process-gone", { nope: true });
    off();
    await reload?.();
    expect(frames.map(({ method, args }) => ({ method, args }))).toEqual([
      { method: "reload", args: { window: "editor" } },
    ]);
  });
});
