import { describe, expect, test } from "bun:test";
import { getCallContext, parseCallContext, runWithCallContext } from "./context";

const signal = new AbortController().signal;
const ctx = {
  window: "main",
  webview: "main",
  origin: "https://remote.test",
  isAppContent: false,
  signal,
};
const wire = { window: "main", webview: "main", origin: "https://remote.test" };

describe("call context", () => {
  test("is undefined outside a call", () => {
    expect(getCallContext()).toBeUndefined();
  });

  test("propagates across await, timers and microtasks", async () => {
    const seen: unknown[] = [];
    await runWithCallContext(ctx, async () => {
      seen.push(getCallContext());
      await Promise.resolve();
      seen.push(getCallContext());
      await new Promise((r) => setTimeout(r, 5));
      seen.push(getCallContext());
      await Promise.all([Bun.sleep(1).then(() => seen.push(getCallContext()))]);
    });
    expect(seen).toEqual([ctx, ctx, ctx, ctx]);
    expect(getCallContext()).toBeUndefined();
  });

  test("ctx absent gives undefined, even inside another caller's scope", async () => {
    const inner = await runWithCallContext(ctx, () =>
      runWithCallContext(undefined, async () => {
        await Promise.resolve();
        return getCallContext();
      }),
    );
    expect(inner).toBeUndefined();
  });

  test("concurrent calls do not leak into each other", async () => {
    const a = { ...ctx, window: "a" };
    const b = { ...ctx, window: "b" };
    const read = async () => {
      await Bun.sleep(2);
      return getCallContext()?.window;
    };
    const [ra, rb] = await Promise.all([runWithCallContext(a, read), runWithCallContext(b, read)]);
    expect([ra, rb]).toEqual(["a", "b"]);
  });

  test("parseCallContext accepts the documented shape and rejects the rest", () => {
    expect(parseCallContext(wire, signal)).toEqual(ctx);
    expect(parseCallContext({ ...wire, origin: null }, signal)).toEqual({
      ...ctx,
      origin: null,
      isAppContent: true,
    });
    expect(parseCallContext(undefined, signal)).toBeUndefined();
    expect(parseCallContext(null, signal)).toBeUndefined();
    expect(parseCallContext("main", signal)).toBeUndefined();
    expect(parseCallContext({ window: "main" }, signal)).toBeUndefined();
    expect(parseCallContext({ ...wire, webview: 1 }, signal)).toBeUndefined();
  });

  test("a host-sent isAppContent or signal never overrides the runtime's", () => {
    const parsed = parseCallContext({ ...wire, isAppContent: true, signal: "x" }, signal);
    expect(parsed?.isAppContent).toBe(false);
    expect(parsed?.signal).toBe(signal);
  });
});
