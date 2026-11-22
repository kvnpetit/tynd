import { afterEach, describe, expect, test } from "bun:test";
import {
  clearMiddleware,
  type MiddlewareContext,
  runMiddleware,
  useMiddleware,
} from "./middleware";

afterEach(clearMiddleware);

const call = (fn = "f", args: unknown[] = []): MiddlewareContext => ({
  fn,
  args,
  caller: undefined,
});

describe("middleware chain", () => {
  test("no middleware: runs the final handler", async () => {
    expect(await runMiddleware(call(), () => 42)).toBe(42);
  });

  test("runs in registration order, first registered outermost", async () => {
    const order: string[] = [];
    useMiddleware(async (_c, next) => {
      order.push("a:in");
      const r = await next();
      order.push("a:out");
      return r;
    });
    useMiddleware(async (_c, next) => {
      order.push("b:in");
      const r = await next();
      order.push("b:out");
      return r;
    });
    const r = await runMiddleware(call(), () => {
      order.push("handler");
      return "ok";
    });
    expect(r).toBe("ok");
    expect(order).toEqual(["a:in", "b:in", "handler", "b:out", "a:out"]);
  });

  test("a throw rejects the call and skips the handler", async () => {
    let ran = false;
    useMiddleware(() => {
      throw new Error("nope");
    });
    await expect(
      runMiddleware(call(), () => {
        ran = true;
      }),
    ).rejects.toThrow("nope");
    expect(ran).toBe(false);
  });

  test("a rejection after next() propagates", async () => {
    useMiddleware(async (_c, next) => {
      await next();
      throw new Error("post");
    });
    await expect(runMiddleware(call(), () => 1)).rejects.toThrow("post");
  });
});

describe("middleware short-circuit and arguments", () => {
  test("not calling next() short-circuits with the returned value", async () => {
    let ran = false;
    useMiddleware(() => "cached");
    const r = await runMiddleware(call(), () => {
      ran = true;
    });
    expect(r).toBe("cached");
    expect(ran).toBe(false);
  });

  test("middleware can replace arguments and transform the result", async () => {
    useMiddleware(async (c, next) => {
      c.args = [...c.args, "extra"];
      return `${await next()}!`;
    });
    const c = call("f", ["x"]);
    const r = await runMiddleware(c, () => c.args.join(","));
    expect(r).toBe("x,extra!");
  });

  test("calling next() twice is rejected", async () => {
    useMiddleware(async (_c, next) => {
      await next();
      return next();
    });
    await expect(runMiddleware(call(), () => 1)).rejects.toThrow("more than once");
  });

  test("rejects non-function middleware", () => {
    expect(() => useMiddleware("x" as never)).toThrow(TypeError);
  });
});
