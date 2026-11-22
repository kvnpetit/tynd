import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, test } from "bun:test";
import { tts } from "./client";

// The core bridge reads `window.__tynd__`; in a webview `window` is `globalThis`.
const previousWindow = (globalThis as { window?: unknown }).window;
beforeAll(() => {
  Object.assign(globalThis, { window: globalThis });
});
afterAll(() => {
  Object.assign(globalThis, { window: previousWindow });
});

type Call = { method: string; args: Record<string, unknown> | null };

let calls: Call[];

beforeEach(() => {
  calls = [];
  (globalThis as Record<string, unknown>).__tynd__ = {
    os_call: async (_api: string, method: string, args: Record<string, unknown> | null) => {
      calls.push({ method: method.replace(/^tts:/, ""), args });
      return method.endsWith("isSpeaking") ? true : undefined;
    },
    on: () => () => undefined,
  };
});

afterEach(() => {
  (globalThis as Record<string, unknown>).__tynd__ = undefined;
});

describe("tts client", () => {
  test("speak forwards text and normalises omitted options to null", async () => {
    await tts.speak("hello");
    await tts.speak("hi", { rate: 1, volume: 0.8 });
    expect(calls).toEqual([
      { method: "speak", args: { text: "hello", opts: null } },
      {
        method: "speak",
        args: { text: "hi", opts: { voice: null, rate: 1, pitch: null, volume: 0.8 } },
      },
    ]);
  });

  test("stop and isSpeaking call the matching commands", async () => {
    await tts.stop();
    expect(await tts.isSpeaking()).toBe(true);
    expect(calls.map((c) => c.method)).toEqual(["stop", "isSpeaking"]);
  });
});
