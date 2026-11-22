import { afterEach, describe, expect, spyOn, test } from "bun:test";
import { app } from "./index";
import { lifecycleFlags } from "./lifecycle";

afterEach(() => {
  lifecycleFlags.readyFired = false;
  lifecycleFlags.closeFired = false;
});

describe("late lifecycle handlers", () => {
  test("a throwing or rejecting onReady/onClose handler registered after the event is isolated", async () => {
    const err = spyOn(process.stderr, "write").mockImplementation((() => true) as never);
    const unhandled: unknown[] = [];
    const onUnhandled = (e: unknown) => unhandled.push(e);
    process.on("unhandledRejection", onUnhandled);
    try {
      lifecycleFlags.readyFired = true;
      lifecycleFlags.closeFired = true;
      let ran = 0;
      app.onReady(() => {
        ran++;
        throw new Error("late ready");
      });
      app.onReady((async () => {
        ran++;
        throw new Error("late async ready");
      }) as () => void);
      app.onClose(() => {
        ran++;
        throw new Error("late close");
      });
      await new Promise((r) => setTimeout(r, 20));
      expect(ran).toBe(3);
      expect(unhandled).toEqual([]);
    } finally {
      process.off("unhandledRejection", onUnhandled);
      err.mockRestore();
    }
  });
});
