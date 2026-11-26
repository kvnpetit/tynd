import { describe, expect, test } from "bun:test";
import { WaitForTimeoutError, waitFor } from "./wait.ts";

describe("waitFor", () => {
  test("resolves with the first truthy value", async () => {
    let calls = 0;
    const value = await waitFor(() => (++calls >= 3 ? "ready" : ""), { interval: 1 });
    expect(value).toBe("ready");
    expect(calls).toBe(3);
  });

  test("supports async predicates", async () => {
    let calls = 0;
    const value = await waitFor(async () => ++calls > 1 && { id: 7 }, { interval: 1 });
    expect(value).toEqual({ id: 7 });
  });

  test("treats thrown errors as not ready and reports the last one on timeout", async () => {
    const error = await waitFor(
      () => {
        throw new Error("no window yet");
      },
      { timeout: 30, interval: 5, message: "main window" },
    ).catch((caught: unknown) => caught);
    expect(error).toBeInstanceOf(WaitForTimeoutError);
    expect((error as WaitForTimeoutError).message).toContain("main window");
    expect((error as WaitForTimeoutError).message).toContain("no window yet");
  });

  test("times out on a predicate that stays falsy", async () => {
    const started = Date.now();
    await expect(waitFor(() => false, { timeout: 40, interval: 5 })).rejects.toThrow(
      "not met within 40 ms",
    );
    expect(Date.now() - started).toBeGreaterThanOrEqual(35);
  });

  test("checks once even with a zero timeout", async () => {
    expect(await waitFor(() => 1, { timeout: 0 })).toBe(1);
  });
});
