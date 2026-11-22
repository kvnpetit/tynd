import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, test } from "bun:test";
import { llm } from "./client";

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
/** Return a value (or promise) for a plugin method; `startStream` is where tests emit events. */
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
      const name = method.replace(/^llm:/, "");
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

async function collect(iterable: AsyncIterable<string>): Promise<string[]> {
  const out: string[] = [];
  for await (const token of iterable) out.push(token);
  return out;
}

describe("llm client", () => {
  test("load fills null defaults", async () => {
    respond = () => ({ id: 1, path: "/m.gguf", contextSize: 2048 });
    await llm.load({ path: "/m.gguf" });
    expect(calls[0]).toEqual({
      method: "load",
      args: { path: "/m.gguf", contextSize: null, gpuLayers: null },
    });
  });

  test("generate installs listeners before startStream and yields tokens in order", async () => {
    let listenersAtStart = 0;
    respond = (method) => {
      if (method === "generate") return { stream: 5 };
      if (method === "startStream") {
        listenersAtStart = listenerCount();
        queueMicrotask(() => {
          emit("llm:token", { stream: 99, token: "other" });
          emit("llm:token", { stream: 5, token: "Hel" });
          emit("llm:token", { stream: 5, token: "lo" });
          emit("llm:end", { stream: 5, ok: true });
        });
      }
      return;
    };
    const tokens = await collect(llm.generate(1, "hi"));
    expect(tokens).toEqual(["Hel", "lo"]);
    expect(listenersAtStart).toBe(2);
    expect(methods()).toEqual(["generate", "startStream"]);
    expect(calls[1]?.args).toEqual({ streamId: 5 });
    expect(listenerCount()).toBe(0);
  });
});

describe("llm client streaming", () => {
  test("generate rejects with the host error when the stream ends with ok=false", async () => {
    respond = (method) => {
      if (method === "generate") return { stream: 7 };
      if (method === "startStream") {
        queueMicrotask(() =>
          emit("llm:end", { stream: 7, ok: false, error: "stream expired: no startStream" }),
        );
      }
      return;
    };
    await expect(collect(llm.generate(1, "hi"))).rejects.toThrow("stream expired");
    expect(listenerCount()).toBe(0);
  });

  test("an already aborted signal cancels without ever starting the stream", async () => {
    respond = (method) => (method === "generate" ? { stream: 3 } : undefined);
    const controller = new AbortController();
    controller.abort();
    const tokens = await collect(llm.generate(1, "hi", { signal: controller.signal }));
    expect(tokens).toEqual([]);
    expect(methods()).toEqual(["generate", "cancel"]);
    expect(calls[1]?.args).toEqual({ streamId: 3 });
    expect(listenerCount()).toBe(0);
  });

  test("breaking out of the loop cancels the native stream", async () => {
    respond = (method) => {
      if (method === "generate") return { stream: 4 };
      if (method === "startStream") {
        queueMicrotask(() => emit("llm:token", { stream: 4, token: "a" }));
      }
      return;
    };
    for await (const token of llm.generate(1, "hi")) {
      expect(token).toBe("a");
      break;
    }
    expect(methods()).toEqual(["generate", "startStream", "cancel"]);
    expect(listenerCount()).toBe(0);
  });
});

describe("llm client cancellation", () => {
  test("a failing startStream surfaces the error and cancels the reservation", async () => {
    respond = (method) => {
      if (method === "generate") return { stream: 8 };
      if (method === "startStream") throw new Error("worker unavailable");
      return;
    };
    await expect(collect(llm.generate(1, "hi"))).rejects.toThrow("worker unavailable");
    expect(methods()).toEqual(["generate", "startStream", "cancel"]);
    expect(listenerCount()).toBe(0);
  });

  test("a cancel that fails is swallowed by the iterator cleanup", async () => {
    respond = (method) => {
      if (method === "generate") return { stream: 9 };
      if (method === "cancel") throw new Error("gone");
      return;
    };
    const controller = new AbortController();
    controller.abort();
    await expect(collect(llm.generate(1, "hi", { signal: controller.signal }))).resolves.toEqual(
      [],
    );
  });

  test("generate forwards sampling options but not the signal", async () => {
    respond = (method) => {
      if (method === "generate") return { stream: 2 };
      if (method === "startStream") {
        queueMicrotask(() => emit("llm:end", { stream: 2, ok: true }));
      }
      return;
    };
    const controller = new AbortController();
    await collect(llm.generate(1, "hi", { maxTokens: 8, stop: ["\n"], signal: controller.signal }));
    expect(calls[0]?.args).toEqual({
      modelId: 1,
      prompt: "hi",
      opts: { maxTokens: 8, stop: ["\n"] },
    });
  });
});

describe("llm client one-shot and handles", () => {
  test("complete without a signal uses the synchronous native arm", async () => {
    respond = (method) => (method === "complete" ? "full text" : undefined);
    await expect(llm.complete(1, "hi", { maxTokens: 4 })).resolves.toBe("full text");
    expect(calls).toEqual([
      { method: "complete", args: { modelId: 1, prompt: "hi", opts: { maxTokens: 4 } } },
    ]);
  });

  test("complete with a signal drains generate", async () => {
    respond = (method) => {
      if (method === "generate") return { stream: 6 };
      if (method === "startStream") {
        queueMicrotask(() => {
          emit("llm:token", { stream: 6, token: "foo" });
          emit("llm:token", { stream: 6, token: "bar" });
          emit("llm:end", { stream: 6, ok: true });
        });
      }
      return;
    };
    const controller = new AbortController();
    await expect(llm.complete(1, "hi", { signal: controller.signal })).resolves.toBe("foobar");
    expect(methods()).toEqual(["generate", "startStream"]);
  });

  test("cancel, unload and list map to native calls", async () => {
    respond = (method) => (method === "list" ? [] : undefined);
    await llm.cancel(11);
    await llm.unload(2);
    await expect(llm.list()).resolves.toEqual([]);
    expect(calls).toEqual([
      { method: "cancel", args: { streamId: 11 } },
      { method: "unload", args: { modelId: 2 } },
      { method: "list", args: null },
    ]);
  });
});
