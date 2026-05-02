import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { workers } from "./workers";

// `workers.spawn` prefers Bun Workers when `globalThis.Worker` is a
// function — but those Workers actually load a `data:` URL and would run
// real code. To exercise the IPC fallback (which we can mock) we strip
// `Worker` from the global for the duration of these tests.
const originalWorker = (globalThis as { Worker?: unknown }).Worker;

type OsCall = { api: string; method: string; args: unknown };

function mountShim(): { osCalls: OsCall[] } {
  const osCalls: OsCall[] = [];
  (globalThis as unknown as { window: unknown }).window = {
    __tynd__: {
      os_call(api: string, method: string, args: unknown) {
        osCalls.push({ api, method, args });
        if (api === "workers" && method === "spawn") {
          return Promise.resolve({ id: 1 });
        }
        return Promise.resolve(null);
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
  return { osCalls };
}

beforeEach(() => {
  // Force the IPC fallback path so we can observe the `workers.spawn`
  // payload without spinning a real Worker. Assigning `undefined` is
  // enough — `isBunWorkerAvailable()` checks `typeof Worker === "function"`.
  (globalThis as { Worker?: unknown }).Worker = undefined;
});

afterEach(() => {
  if (originalWorker !== undefined) {
    (globalThis as { Worker?: unknown }).Worker = originalWorker;
  }
  (globalThis as { window?: unknown }).window = undefined;
});

describe("workers.spawn — input validation", () => {
  test("accepts a plain function", async () => {
    mountShim();
    await expect(workers.spawn(() => 42)).resolves.toMatchObject({ id: 1 });
  });

  test("rejects a non-function (string is the only legal alt form)", async () => {
    mountShim();
    // Numbers / objects / undefined all hit the type guard.
    await expect(
      // biome-ignore lint/suspicious/noExplicitAny: testing runtime guard
      workers.spawn(42 as any),
    ).rejects.toThrow(/expected a function/);
    await expect(
      // biome-ignore lint/suspicious/noExplicitAny: testing runtime guard
      workers.spawn(null as any),
    ).rejects.toThrow(/expected a function/);
  });

  test("rejects native [native code] functions", async () => {
    mountShim();
    await expect(workers.spawn(Math.max as unknown as (input: unknown) => unknown)).rejects.toThrow(
      /native functions/,
    );
  });

  test("string form bypasses validation (legacy path)", async () => {
    const { osCalls } = mountShim();
    await workers.spawn("(input) => input * 2");
    const spawnCall = osCalls.find((c) => c.method === "spawn");
    expect((spawnCall?.args as { script: string }).script).toBe("(input) => input * 2");
  });
});

describe("workers.spawn — pure functions are accepted", () => {
  test("functions using only globals + args do not throw", async () => {
    mountShim();
    // Uses only `console` + `Math` (both in KNOWN_WORKER_GLOBALS) + the arg.
    const fn = new Function(
      "return (input) => { console.log(input); return Math.max(input, 0); }",
    )() as (input: unknown) => unknown;
    await expect(workers.spawn(fn)).resolves.toMatchObject({ id: 1 });
  });

  test("functions referencing closure vars still spawn (warn-only, non-fatal)", async () => {
    // Closure detection is dev-mode advisory — it must never block spawn,
    // since the heuristic has false-positives on minified / bundled code.
    mountShim();
    const fn = new Function("return (input) => input + outerVar")() as (input: unknown) => unknown;
    await expect(workers.spawn(fn)).resolves.toMatchObject({ id: 1 });
  });
});
