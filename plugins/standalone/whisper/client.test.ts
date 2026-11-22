import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, test } from "bun:test";
import { whisper } from "./client";

// The core bridge reads `window.__tynd__`; in a webview `window` is `globalThis`.
const previousWindow = (globalThis as { window?: unknown }).window;
beforeAll(() => {
  Object.assign(globalThis, { window: globalThis });
});
afterAll(() => {
  Object.assign(globalThis, { window: previousWindow });
});

type Handler = (payload: unknown) => void;
type Call = { method: string; args: Record<string, unknown> | null };

let calls: Call[];
let listeners: Map<string, Set<Handler>>;
let respond: (method: string, args: Record<string, unknown> | null) => unknown;

function emit(event: string, payload: unknown): void {
  for (const handler of [...(listeners.get(event) ?? [])]) handler(payload);
}

function listenerCount(): number {
  let total = 0;
  for (const set of listeners.values()) total += set.size;
  return total;
}

function methods(): string[] {
  return calls.map((c) => c.method);
}

beforeEach(() => {
  calls = [];
  listeners = new Map();
  respond = () => undefined;
  (globalThis as Record<string, unknown>).__tynd__ = {
    os_call: async (_api: string, method: string, args: Record<string, unknown> | null) => {
      const name = method.replace(/^whisper:/, "");
      calls.push({ method: name, args });
      return respond(name, args);
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
  (globalThis as Record<string, unknown>).__tynd__ = undefined;
});

async function collect<T>(iterable: AsyncIterable<T>): Promise<T[]> {
  const out: T[] = [];
  for await (const item of iterable) out.push(item);
  return out;
}

/** Respond to a streaming `transcribe` with the given segments, then end. */
function streamingResponder(
  stream: number,
  segments: { startMs: number; endMs: number; text: string }[],
  onStart?: () => void,
): void {
  respond = (method) => {
    if (method === "transcribe") return { stream };
    if (method === "startStream") {
      onStart?.();
      queueMicrotask(() => {
        for (const segment of segments) emit("whisper:segment", { stream, segment });
        emit("whisper:end", { stream, ok: true });
      });
    }
    return;
  };
}

describe("whisper client", () => {
  test("load defaults gpu to true", async () => {
    respond = () => ({ id: 1, path: "/m.bin", sampleRate: 16000, language: "multi" });
    await whisper.load({ path: "/m.bin" });
    expect(calls[0]).toEqual({ method: "load", args: { path: "/m.bin", gpu: true } });
    await whisper.load({ path: "/m.bin", gpu: false });
    expect(calls[1]?.args).toEqual({ path: "/m.bin", gpu: false });
  });

  test("transcribe sends base64 f32 PCM and null format hints", async () => {
    streamingResponder(1, []);
    await collect(whisper.transcribe(2, new Float32Array([0, 0.5, -1])));
    const sent = calls[0]?.args as { samples: string; sampleRate: unknown; channels: unknown };
    expect(sent.sampleRate).toBeNull();
    expect(sent.channels).toBeNull();
    const bytes = Uint8Array.from(atob(sent.samples), (c) => c.charCodeAt(0));
    expect(Array.from(new Float32Array(bytes.buffer))).toEqual([0, 0.5, -1]);
  });

  test("transcribe installs listeners before startStream and yields segments", async () => {
    let listenersAtStart = 0;
    streamingResponder(
      3,
      [
        { startMs: 0, endMs: 500, text: " hello" },
        { startMs: 500, endMs: 900, text: " world" },
      ],
      () => {
        listenersAtStart = listenerCount();
      },
    );
    const segments = await collect(whisper.transcribe(1, new Float32Array(16)));
    expect(segments.map((s) => s.text)).toEqual([" hello", " world"]);
    expect(listenersAtStart).toBe(2);
    expect(methods()).toEqual(["transcribe", "startStream"]);
    expect(listenerCount()).toBe(0);
  });
});

describe("whisper client errors", () => {
  test("transcribe rejects when the stream ends with ok=false (e.g. expired)", async () => {
    respond = (method) => {
      if (method === "transcribe") return { stream: 4 };
      if (method === "startStream") {
        queueMicrotask(() => emit("whisper:end", { stream: 4, ok: false, error: "busy" }));
      }
      return;
    };
    await expect(collect(whisper.transcribe(1, new Float32Array(16)))).rejects.toThrow("busy");
    expect(listenerCount()).toBe(0);
  });

  test("an aborted signal cancels without starting", async () => {
    respond = (method) => (method === "transcribe" ? { stream: 5 } : undefined);
    const controller = new AbortController();
    controller.abort();
    const out = await collect(
      whisper.transcribe(1, new Float32Array(16), { signal: controller.signal }),
    );
    expect(out).toEqual([]);
    expect(methods()).toEqual(["transcribe", "cancel"]);
    expect(calls[1]?.args).toEqual({ streamId: 5 });
  });

  test("breaking out of the loop cancels the native stream", async () => {
    // Keep the stream open: one segment, no end event.
    respond = (method) => {
      if (method === "transcribe") return { stream: 6 };
      if (method === "startStream") {
        queueMicrotask(() =>
          emit("whisper:segment", { stream: 6, segment: { startMs: 0, endMs: 1, text: "a" } }),
        );
      }
      return;
    };
    for await (const segment of whisper.transcribe(1, new Float32Array(16))) {
      expect(segment.text).toBe("a");
      break;
    }
    expect(methods()).toEqual(["transcribe", "startStream", "cancel"]);
    expect(listenerCount()).toBe(0);
  });
});

describe("whisper client one-shot", () => {
  test("transcribeAll without a signal uses the native arm", async () => {
    respond = (method) => (method === "transcribeAll" ? "text" : undefined);
    await expect(whisper.transcribeAll(1, new Float32Array(4), { language: "fr" })).resolves.toBe(
      "text",
    );
    expect(calls[0]?.args).toMatchObject({
      modelId: 1,
      sampleRate: null,
      channels: null,
      opts: { language: "fr" },
    });
  });

  test("transcribeAll with a signal concatenates streamed segments", async () => {
    streamingResponder(8, [
      { startMs: 0, endMs: 1, text: "a" },
      { startMs: 1, endMs: 2, text: "b" },
    ]);
    const controller = new AbortController();
    await expect(
      whisper.transcribeAll(1, new Float32Array(4), { signal: controller.signal }),
    ).resolves.toBe("ab");
  });

  test("transcribeStream validates windowMs", async () => {
    async function* audio(): AsyncGenerator<Float32Array> {
      yield new Float32Array(10);
    }
    for (const windowMs of [0, -1, Number.NaN, 30_001]) {
      await expect(collect(whisper.transcribeStream(1, audio(), { windowMs }))).rejects.toThrow(
        RangeError,
      );
    }
    expect(calls).toEqual([]);
  });
});

describe("whisper client windows", () => {
  test("transcribeStream cuts windows and offsets segment timestamps", async () => {
    let next = 100;
    respond = (method) => {
      if (method === "transcribe") {
        next += 1;
        return { stream: next };
      }
      if (method === "startStream") {
        const stream = next;
        queueMicrotask(() => {
          emit("whisper:segment", {
            stream,
            segment: { startMs: 0, endMs: 250, text: `w${stream}` },
          });
          emit("whisper:end", { stream, ok: true });
        });
      }
      return;
    };
    // 1 s windows at 16 kHz; 2.5 s of audio fed in odd-sized chunks => 3 windows (1 s, 1 s, 0.5 s).
    async function* audio(): AsyncGenerator<Float32Array> {
      yield new Float32Array(10_000);
      yield new Float32Array(20_000);
      yield new Float32Array(10_000);
    }
    const segments = await collect(whisper.transcribeStream(1, audio(), { windowMs: 1000 }));
    expect(segments.map((s) => [s.startMs, s.endMs])).toEqual([
      [0, 250],
      [1000, 1250],
      [2000, 2250],
    ]);
    const windowSizes = calls
      .filter((c) => c.method === "transcribe")
      .map((c) => {
        const encoded = (c.args as { samples: string }).samples;
        return atob(encoded).length / 4;
      });
    expect(windowSizes).toEqual([16_000, 16_000, 8_000]);
  });
});

describe("whisper client handles", () => {
  test("detectLanguage, cancel, unload and list map to native calls", async () => {
    respond = (method) => {
      if (method === "detectLanguage") return { language: "en", confidence: 0.93 };
      if (method === "list") return [];
      return;
    };
    await expect(whisper.detectLanguage(1, new Float32Array(4))).resolves.toEqual({
      language: "en",
      confidence: 0.93,
    });
    await whisper.cancel(3);
    await whisper.unload(1);
    await whisper.list();
    expect(methods()).toEqual(["detectLanguage", "cancel", "unload", "list"]);
    expect(calls[1]?.args).toEqual({ streamId: 3 });
  });

  test("transcribeStream stops reading audio once the signal aborts", async () => {
    respond = (method) => (method === "transcribe" ? { stream: 1 } : undefined);
    const controller = new AbortController();
    controller.abort();
    let pulled = 0;
    async function* audio(): AsyncGenerator<Float32Array> {
      for (let i = 0; i < 10; i++) {
        pulled += 1;
        yield new Float32Array(100);
      }
    }
    const segments = await collect(
      whisper.transcribeStream(1, audio(), { windowMs: 1000, signal: controller.signal }),
    );
    expect(segments).toEqual([]);
    expect(pulled).toBe(1);
    expect(methods()).not.toContain("transcribe");
  });
});
