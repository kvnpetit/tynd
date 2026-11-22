import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, test } from "bun:test";
import { screenRec } from "./client";

// The core bridge reads `window.__tynd__`; in a webview `window` is `globalThis`.
const previousWindow = (globalThis as { window?: unknown }).window;
beforeAll(() => {
  Object.assign(globalThis, { window: globalThis });
});
afterAll(() => {
  Object.assign(globalThis, { window: previousWindow });
});

type Call = { target: string; args: unknown };

const HANDLE = { id: 4, startedAt: 1, output: "out.mjpeg" };

let calls: Call[];
let responses: Record<string, unknown>;
const globals = globalThis as { __tynd__?: unknown };
const original = globals.__tynd__;

beforeEach(() => {
  calls = [];
  responses = { "screen-rec:start": HANDLE };
  globals.__tynd__ = {
    os_call: async (_ns: string, target: string, args: unknown) => {
      calls.push({ target, args });
      return responses[target] ?? null;
    },
  };
});

afterEach(() => {
  globals.__tynd__ = original;
});

describe("screen-rec client", () => {
  test("start fills every default mirroring the Rust RecordingOpts", async () => {
    const handle = await screenRec.start({ output: "out.mjpeg" });
    expect(calls).toEqual([
      {
        target: "screen-rec:start",
        args: {
          opts: {
            fps: 30,
            region: null,
            monitor: null,
            audio: false,
            format: null,
            output: "out.mjpeg",
            codec: "mjpeg",
            encoder: "auto",
            bitrate: 8000,
            preset: "fast",
          },
        },
      },
    ]);
    expect(handle).toEqual(HANDLE);
  });

  test("start keeps caller overrides and only fills the rest", async () => {
    await screenRec.start({
      output: "o.mp4",
      fps: 60,
      codec: "h264",
      monitor: 2,
      region: { x: 1, y: 2, width: 3, height: 4 },
      bitrate: 1000,
    });
    const sent = ((calls[0]?.args ?? {}) as { opts: Record<string, unknown> }).opts;
    expect(sent).toMatchObject({
      output: "o.mp4",
      fps: 60,
      codec: "h264",
      monitor: 2,
      region: { x: 1, y: 2, width: 3, height: 4 },
      bitrate: 1000,
      encoder: "auto",
      preset: "fast",
      audio: false,
    });
  });
});

describe("screen-rec client errors and controls", () => {
  test("a rejected start propagates the native error", async () => {
    globals.__tynd__ = {
      os_call: async () => {
        throw new Error("screen-rec: screen-capture permission denied");
      },
    };
    await expect(screenRec.start({ output: "o.mjpeg" })).rejects.toThrow("permission denied");
  });

  test("stop, pause and resume forward the recording id", async () => {
    await screenRec.stop({ recordingId: 4 });
    await screenRec.pause({ recordingId: 4 });
    await screenRec.resume({ recordingId: 4 });
    expect(calls).toEqual([
      { target: "screen-rec:stop", args: { recordingId: 4 } },
      { target: "screen-rec:pause", args: { recordingId: 4 } },
      { target: "screen-rec:resume", args: { recordingId: 4 } },
    ]);
  });
});
