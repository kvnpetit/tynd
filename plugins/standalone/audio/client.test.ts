import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, test } from "bun:test";
import { audio } from "./client";

// The core bridge reads `window.__tynd__`; in a webview `window` is `globalThis`.
const previousWindow = (globalThis as { window?: unknown }).window;
beforeAll(() => {
  Object.assign(globalThis, { window: globalThis });
});
afterAll(() => {
  Object.assign(globalThis, { window: previousWindow });
});

type Handler = (payload: unknown) => void;

let calls: { target: string; args: unknown }[];
let listeners: Map<string, Set<Handler>>;
let startDelay: Promise<void> | null;
const globals = globalThis as { __tynd__?: unknown };
const original = globals.__tynd__;

function emit(event: string, payload: unknown): void {
  for (const handler of [...(listeners.get(event) ?? [])]) handler(payload);
}

function f32(...values: number[]): string {
  return Buffer.from(new Float32Array(values).buffer).toString("base64");
}

function samples(id: number, ...values: number[]) {
  return { id, samples: f32(...values), timestamp: 1 };
}

beforeEach(() => {
  calls = [];
  listeners = new Map();
  startDelay = null;
  globals.__tynd__ = {
    os_call: async (_ns: string, target: string, args: unknown) => {
      calls.push({ target, args });
      if (target === "audio:startCapture") {
        await startDelay;
        return { id: 3 };
      }
      return null;
    },
    os_on: (event: string, handler: Handler) => {
      const set = listeners.get(event) ?? new Set<Handler>();
      set.add(handler);
      listeners.set(event, set);
      return () => set.delete(handler);
    },
  };
});

afterEach(() => {
  globals.__tynd__ = original;
});

const flush = () => new Promise((resolve) => setTimeout(resolve, 0));

describe("audio capture client", () => {
  test("yields chunks for its own stream id only", async () => {
    const stream = audio.startCapture();
    await stream.streamId;
    emit("audio:samples", samples(4, 9));
    emit("audio:samples", samples(3, 0.5, 0.25));
    const first = await stream[Symbol.asyncIterator]().next();
    expect(first.done).toBe(false);
    expect([...(first.value as { samples: Float32Array }).samples]).toEqual([0.5, 0.25]);
    await stream.stop();
  });

  test("a waiting next() resolves when samples arrive", async () => {
    const stream = audio.startCapture();
    await stream.streamId;
    const pending = stream[Symbol.asyncIterator]().next();
    emit("audio:samples", samples(3, 1));
    expect((await pending).done).toBe(false);
    await stream.stop();
  });
});

describe("audio capture client start", () => {
  test("samples emitted before the start reply are not lost", async () => {
    let release: () => void = () => undefined;
    startDelay = new Promise<void>((resolve) => {
      release = resolve;
    });
    const stream = audio.startCapture();
    await flush();
    emit("audio:samples", samples(3, 0.75));
    emit("audio:samples", samples(8, 0.1));
    release();
    await stream.streamId;
    const first = await stream[Symbol.asyncIterator]().next();
    expect(first.done).toBe(false);
    expect([...(first.value as { samples: Float32Array }).samples]).toEqual([0.75]);
    await stream.stop();
  });

  test("a device error ends a waiting next() with a rejection", async () => {
    const stream = audio.startCapture();
    await stream.streamId;
    const pending = stream[Symbol.asyncIterator]().next();
    emit("audio:error", { id: 3, kind: "stream-error", message: "device unplugged" });
    await expect(pending).rejects.toThrow("device unplugged");
    await flush();
    expect(calls.map((c) => c.target)).toContain("audio:stopCapture");
    expect(listeners.get("audio:samples")?.size).toBe(0);
  });

  test("chunks captured before a device error are delivered first", async () => {
    const stream = audio.startCapture();
    await stream.streamId;
    emit("audio:samples", samples(3, 1));
    emit("audio:error", { id: 3, kind: "stream-error", message: "boom" });
    const it = stream[Symbol.asyncIterator]();
    expect((await it.next()).done).toBe(false);
    await expect(it.next()).rejects.toThrow("boom");
  });
});

describe("audio capture client errors", () => {
  test("an error for another stream is ignored", async () => {
    const stream = audio.startCapture();
    await stream.streamId;
    emit("audio:error", { id: 99, kind: "stream-error", message: "other" });
    const pending = stream[Symbol.asyncIterator]().next();
    emit("audio:samples", samples(3, 1));
    expect((await pending).done).toBe(false);
    await stream.stop();
  });

  test("a failed start rejects next() and detaches", async () => {
    (globals.__tynd__ as { os_call: unknown }).os_call = async () => {
      throw new Error("no microphone");
    };
    const stream = audio.startCapture();
    stream.streamId.catch(() => undefined);
    await expect(stream[Symbol.asyncIterator]().next()).rejects.toThrow("no microphone");
    expect(listeners.get("audio:samples")?.size).toBe(0);
  });

  test("stop resolves a pending next() as done and stops natively once", async () => {
    const stream = audio.startCapture();
    await stream.streamId;
    const pending = stream[Symbol.asyncIterator]().next();
    await stream.stop();
    await stream.stop();
    expect((await pending).done).toBe(true);
    expect(calls.filter((c) => c.target === "audio:stopCapture")).toHaveLength(1);
    expect(listeners.get("audio:samples")?.size).toBe(0);
    expect(listeners.get("audio:error")?.size).toBe(0);
  });

  test("breaking out of for-await stops the stream", async () => {
    const stream = audio.startCapture();
    await stream.streamId;
    emit("audio:samples", samples(3, 1));
    for await (const _chunk of stream) break;
    expect(calls.map((c) => c.target)).toContain("audio:stopCapture");
  });
});

describe("audio capture client buffering", () => {
  test("the queue of a never-iterated capture is bounded", async () => {
    const stream = audio.startCapture();
    await stream.streamId;
    for (let i = 0; i < 5000; i++) emit("audio:samples", samples(3, i));
    const it = stream[Symbol.asyncIterator]();
    let count = 0;
    for (;;) {
      const next = await Promise.race([it.next(), flush().then(() => null)]);
      if (next === null || next.done) break;
      count++;
    }
    expect(count).toBe(1024);
    await stream.stop();
  });

  test("startCapture forwards null options when none are given", async () => {
    const stream = audio.startCapture();
    await stream.streamId;
    expect(calls[0]).toEqual({ target: "audio:startCapture", args: { opts: null } });
    await stream.stop();
  });
});
