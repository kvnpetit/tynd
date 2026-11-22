import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import {
  BEFORE_QUIT_TIMEOUT_MS,
  fireBeforeQuit,
  fireBeforeStart,
  fireReady,
  lifecycleFlags,
} from "../lifecycle";
import { type RecordedHook, recordingPlugin } from "./hook-fixture";
import { fireHook } from "./hooks";
import { clearMiddleware, runMiddleware, useMiddleware } from "./middleware";
import { registerPlugin, resetPluginRegistry } from "./plugin-registry";

let log: RecordedHook[];

beforeEach(() => {
  resetPluginRegistry();
  clearMiddleware();
  log = [];
});
afterEach(() => {
  resetPluginRegistry();
  clearMiddleware();
  lifecycleFlags.readyFired = false;
});

describe("plugin hook registry", () => {
  test("plugins without hooks do not register anything", async () => {
    registerPlugin({ __tynd_plugin__: true, name: "plain", methods: {} });
    await fireHook("app:ready");
    expect(log).toEqual([]);
  });

  test("re-registering a plugin keeps the first hooks only", async () => {
    registerPlugin(recordingPlugin("dup", ["app:ready"], log));
    registerPlugin(recordingPlugin("dup", ["app:ready"], log));
    await fireHook("app:ready");
    expect(log).toHaveLength(1);
  });

  test("hooks fire in plugin registration order", async () => {
    registerPlugin(recordingPlugin("one", ["app:ready"], log));
    registerPlugin(recordingPlugin("two", ["app:ready"], log));
    await fireHook("app:ready");
    expect(log.map((l) => l.plugin)).toEqual(["one", "two"]);
  });

  test("a failing hook is isolated from the other plugins", async () => {
    registerPlugin(
      recordingPlugin("bad", ["app:ready"], log, {
        "app:ready": () => {
          throw new Error("boom");
        },
      }),
    );
    registerPlugin(recordingPlugin("good", ["app:ready"], log));
    await fireHook("app:ready");
    expect(log.map((l) => l.plugin)).toEqual(["bad", "good"]);
  });

  test("per-plugin state persists across hook calls", async () => {
    const states: unknown[] = [];
    registerPlugin({
      __tynd_plugin__: true,
      name: "stateful",
      methods: {},
      hooks: {
        async callHook(_n: never, ...args: never[]) {
          const cx = args[0] as unknown as { state: Record<string, number> };
          cx.state.n = (cx.state.n ?? 0) + 1;
          states.push(cx.state.n);
        },
      },
    });
    await fireHook("app:ready");
    await fireHook("app:beforeQuit");
    expect(states).toEqual([1, 2]);
  });
});

describe("app lifecycle hooks", () => {
  const all = ["app:beforeStart", "app:ready", "app:beforeQuit"];

  test("app:beforeStart receives the app config", async () => {
    registerPlugin(recordingPlugin("p", all, log));
    const config = { window: { title: "T" } };
    await fireBeforeStart(config);
    expect(log).toEqual([{ plugin: "p", name: "app:beforeStart", args: [config] }]);
  });

  test("app:ready fires and onReady handlers still run", async () => {
    registerPlugin(recordingPlugin("p", all, log));
    await fireReady();
    expect(log.map((l) => l.name)).toEqual(["app:ready"]);
    expect(lifecycleFlags.readyFired).toBe(true);
  });

  test("app:beforeQuit fires", async () => {
    registerPlugin(recordingPlugin("p", all, log));
    await fireBeforeQuit();
    expect(log.map((l) => l.name)).toEqual(["app:beforeQuit"]);
  });

  test("app:beforeQuit is bounded by a timeout", async () => {
    expect(BEFORE_QUIT_TIMEOUT_MS).toBeGreaterThan(0);
    registerPlugin(
      recordingPlugin("slow", all, log, {
        "app:beforeQuit": () => new Promise<void>(() => undefined),
      }),
    );
    const started = Date.now();
    await fireBeforeQuit(30);
    expect(Date.now() - started).toBeLessThan(1000);
  });
});

describe("window hooks", () => {
  test("window:created and window:closed carry the label", async () => {
    registerPlugin(recordingPlugin("p", ["window:created", "window:closed"], log));
    await fireHook("window:created", { label: "main" });
    await fireHook("window:closed", { label: "main" });
    expect(log).toEqual([
      { plugin: "p", name: "window:created", args: [{ label: "main" }] },
      { plugin: "p", name: "window:closed", args: [{ label: "main" }] },
    ]);
  });
});

describe("ipc hooks via the middleware chain", () => {
  const call = (fn = "doIt", plugin?: string) => ({
    fn,
    args: [1, 2] as unknown[],
    caller: undefined,
    plugin,
  });

  test("ipc:before then handler then ipc:after with the result", async () => {
    const order: string[] = [];
    registerPlugin(
      recordingPlugin("p", ["ipc:before", "ipc:after"], log, {
        "ipc:before": () => order.push("before"),
        "ipc:after": () => order.push("after"),
      }),
    );
    const result = await runMiddleware(call(), () => {
      order.push("handler");
      return "r";
    });
    expect(result).toBe("r");
    expect(order).toEqual(["before", "handler", "after"]);
    expect(log).toEqual([
      { plugin: "p", name: "ipc:before", args: ["doIt", [1, 2]] },
      { plugin: "p", name: "ipc:after", args: ["doIt", "r"] },
    ]);
  });

  test("ipc:before throwing rejects the call and skips the handler", async () => {
    registerPlugin(
      recordingPlugin("p", ["ipc:before", "ipc:after"], log, {
        "ipc:before": () => {
          throw new Error("rejected by hook");
        },
      }),
    );
    let ran = false;
    await expect(
      runMiddleware(call(), () => {
        ran = true;
      }),
    ).rejects.toThrow("rejected by hook");
    expect(ran).toBe(false);
    expect(log.map((l) => l.name)).toEqual(["ipc:before"]);
  });
});

describe("ipc hooks failure and ordering", () => {
  const call = (fn = "doIt", plugin?: string) => ({
    fn,
    args: [1, 2] as unknown[],
    caller: undefined,
    plugin,
  });

  test("ipc:after does not fire when the handler fails", async () => {
    registerPlugin(recordingPlugin("p", ["ipc:before", "ipc:after"], log));
    await expect(
      runMiddleware(call(), () => {
        throw new Error("handler failed");
      }),
    ).rejects.toThrow("handler failed");
    expect(log.map((l) => l.name)).toEqual(["ipc:before"]);
  });

  test("plugin method calls report the wire command name", async () => {
    registerPlugin(recordingPlugin("p", ["ipc:before"], log));
    await runMiddleware(call("method", "other"), () => null);
    expect(log[0]?.args[0]).toBe("$plugin$other$method");
  });

  test("hooks run outside user middleware", async () => {
    const order: string[] = [];
    registerPlugin(
      recordingPlugin("p", ["ipc:before", "ipc:after"], log, {
        "ipc:before": () => order.push("hook:before"),
        "ipc:after": () => order.push("hook:after"),
      }),
    );
    useMiddleware(async (_c, next) => {
      order.push("mw:in");
      const r = await next();
      order.push("mw:out");
      return r;
    });
    await runMiddleware(call(), () => order.push("handler"));
    expect(order).toEqual(["hook:before", "mw:in", "handler", "mw:out", "hook:after"]);
  });
});
