import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, test } from "bun:test";
import { bytesToBase64 } from "@tynd/core/client/bridge";
import { terminal } from "./client";

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
const globals = globalThis as { __tynd__?: unknown };
const original = globals.__tynd__;

function emit(event: string, payload: unknown): void {
  for (const handler of [...(listeners.get(event) ?? [])]) handler(payload);
}

function data(id: number, text: string): { id: number; data: string } {
  return { id, data: bytesToBase64(new TextEncoder().encode(text)) };
}

const decode = (chunk: Uint8Array) => new TextDecoder().decode(chunk);

beforeEach(() => {
  calls = [];
  listeners = new Map();
  globals.__tynd__ = {
    os_call: async (_ns: string, target: string, args: unknown) => {
      calls.push({ target, args });
      return target === "terminal:spawn" ? { id: 5 } : null;
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

describe("terminal client", () => {
  test("spawn reserves the session then activates it after subscribing", async () => {
    await terminal.spawn({ shell: "sh" });
    expect(calls.map((c) => c.target)).toEqual(["terminal:spawn", "terminal:activate"]);
    expect(calls[1]?.args).toEqual({ id: 5 });
    expect(listeners.get("terminal:data")?.size).toBe(1);
    expect(listeners.get("terminal:exit")?.size).toBe(1);
  });

  test("output emitted before onData is attached is replayed once", async () => {
    const handle = await terminal.spawn();
    emit("terminal:data", data(5, "he"));
    emit("terminal:data", data(5, "llo"));
    const seen: string[] = [];
    handle.onData((chunk) => seen.push(decode(chunk)));
    expect(seen).toEqual(["he", "llo"]);
    emit("terminal:data", data(5, "!"));
    expect(seen).toEqual(["he", "llo", "!"]);
  });

  test("events for another session are ignored", async () => {
    const handle = await terminal.spawn();
    const seen: string[] = [];
    handle.onData((chunk) => seen.push(decode(chunk)));
    emit("terminal:data", data(99, "other"));
    expect(seen).toEqual([]);
  });

  test("output that arrived before a fast exit still reaches a late onData", async () => {
    const handle = await terminal.spawn();
    emit("terminal:data", data(5, "hi\n"));
    emit("terminal:exit", { id: 5, code: 0 });
    const seen: string[] = [];
    handle.onData((chunk) => seen.push(decode(chunk)));
    expect(seen).toEqual(["hi\n"]);
  });

  test("exit detaches the host subscriptions and replays the code to a late onExit", async () => {
    const handle = await terminal.spawn();
    const early: (number | null)[] = [];
    handle.onExit((code) => early.push(code));
    emit("terminal:exit", { id: 5, code: 3 });
    expect(early).toEqual([3]);
    expect(listeners.get("terminal:data")?.size).toBe(0);
    expect(listeners.get("terminal:exit")?.size).toBe(0);

    const late: (number | null)[] = [];
    handle.onExit((code) => late.push(code));
    await Promise.resolve();
    expect(late).toEqual([3]);
  });
});

describe("terminal client lifecycle", () => {
  test("data arriving after exit is dropped", async () => {
    const handle = await terminal.spawn();
    const seen: string[] = [];
    handle.onData((chunk) => seen.push(decode(chunk)));
    emit("terminal:exit", { id: 5, code: 0 });
    emit("terminal:data", data(5, "late"));
    expect(seen).toEqual([]);
  });

  test("the backlog kept while nobody listens is bounded", async () => {
    const handle = await terminal.spawn();
    const block = "x".repeat(256 * 1024);
    for (let i = 0; i < 16; i++) emit("terminal:data", data(5, block));
    let total = 0;
    handle.onData((chunk) => {
      total += chunk.byteLength;
    });
    expect(total).toBeGreaterThan(0);
    expect(total).toBeLessThanOrEqual(1024 * 1024);
  });

  test("kill calls the native kill and drops listeners and backlog", async () => {
    const handle = await terminal.spawn();
    emit("terminal:data", data(5, "pending"));
    await handle.kill();
    expect(calls.at(-1)).toEqual({ target: "terminal:kill", args: { id: 5 } });
    expect(listeners.get("terminal:data")?.size).toBe(0);
    const seen: string[] = [];
    handle.onData((chunk) => seen.push(decode(chunk)));
    expect(seen).toEqual([]);
  });

  test("write sends base64-encoded bytes", async () => {
    const handle = await terminal.spawn();
    await handle.write("ls\n");
    expect(calls.at(-1)).toEqual({
      target: "terminal:write",
      args: { id: 5, data: bytesToBase64(new TextEncoder().encode("ls\n")) },
    });
  });
});
