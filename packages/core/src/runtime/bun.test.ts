import { afterEach, beforeEach, describe, expect, spyOn, test } from "bun:test";
import { parse } from "devalue";
import { resolveHostCall, setHostCallTransport } from "../host";
import { lifecycleFlags, onCloseFns, onReadyFns, setEmitFn } from "../lifecycle";
import {
  fireBeforeStartBounded,
  handleParsedMsg,
  resetShutdownForTests,
  sendHostTrust,
} from "./bun";
import { getCallContext } from "./context";
import { type RecordedHook, recordingPlugin } from "./hook-fixture";
import { clearMiddleware, useMiddleware } from "./middleware";
import { registerPlugin, resetPluginRegistry } from "./plugin-registry";
import { encode } from "./wire";

const ENTRY = new URL("./bun.fixture.ts", import.meta.url).pathname.replace(/^\/([A-Za-z]:)/, "$1");
const CTX = { window: "main", webview: "child", origin: null };
/** What a handler sees for `CTX`, minus the signal. */
const SEEN = { ...CTX, isAppContent: true, aborted: false };

interface Frame {
  type: string;
  id?: string;
  ok?: boolean;
  v?: string;
  error?: string;
}

let frames: Frame[];
let restore: () => void;
let hookLog: RecordedHook[];

beforeEach(() => {
  frames = [];
  hookLog = [];
  resetPluginRegistry();
  clearMiddleware();
  resetShutdownForTests();
  const spy = spyOn(process.stdout, "write").mockImplementation(((chunk: unknown) => {
    for (const line of String(chunk).split("\n")) {
      if (line.trim()) frames.push(JSON.parse(line) as Frame);
    }
    return true;
  }) as never);
  restore = () => spy.mockRestore();
});
afterEach(() => {
  restore();
  resetPluginRegistry();
  clearMiddleware();
  lifecycleFlags.readyFired = false;
  lifecycleFlags.closeFired = false;
});

const send = (msg: Record<string, unknown>) => handleParsedMsg(msg, ENTRY);
const call = (id: string, fn: string, args: unknown[] = [], ctx?: unknown) =>
  send({ type: "call", id, fn, a: encode(args), ...(ctx === undefined ? {} : { ctx }) });
const result = (id: string): unknown => {
  const f = frames.find((x) => x.id === id && x.type === "return");
  if (!f?.ok) throw new Error(`call ${id} failed: ${f?.error}`);
  return parse(f.v ?? "");
};
const errorOf = (id: string): string => {
  const f = frames.find((x) => x.id === id && x.type === "return");
  if (!f || f.ok) throw new Error(`call ${id} did not fail`);
  return f.error ?? "";
};

describe("call context over the wire", () => {
  test("handler sees msg.ctx across awaits", async () => {
    await call("1", "whoCalled", [], CTX);
    expect(result("1")).toEqual(SEEN);
  });

  test("ctx absent gives undefined", async () => {
    await call("2", "whoCalled");
    expect(result("2")).toBeNull();
  });

  test("malformed ctx is ignored, the call still succeeds", async () => {
    await call("3", "add", [1, 2], { window: 7 });
    expect(result("3")).toBe(3);
    await call("4", "whoCalled", [], "nope");
    expect(result("4")).toBeNull();
  });

  test("context survives stream iteration", async () => {
    await call("5", "countdown", [], CTX);
    const yields = frames.filter((f) => f.id === "5" && f.type === "yield");
    expect(yields.map((f) => parse(f.v ?? ""))).toEqual(["main", "main"]);
    expect(result("5")).toBe("done");
  });

  test("JS plugin methods run in the caller's scope", async () => {
    registerPlugin({
      __tynd_plugin__: true,
      name: "ctxp",
      methods: {
        who: async () => {
          await Promise.resolve();
          const { signal: _signal, ...rest } = getCallContext() ?? ({} as never);
          return { ...rest, aborted: false };
        },
      },
    });
    await call("6", "$plugin$ctxp$who", [], CTX);
    expect(result("6")).toEqual(SEEN);
  });
});

describe("middleware over the wire", () => {
  test("wraps user functions with fn, args and caller, in registration order", async () => {
    const seen: unknown[] = [];
    useMiddleware(async (ctx, next) => {
      const { signal: _signal, ...caller } = ctx.caller ?? ({} as never);
      seen.push(["a", ctx.fn, ctx.args, { ...caller, aborted: false }, ctx.plugin]);
      return next();
    });
    useMiddleware(async (_ctx, next) => {
      seen.push("b");
      return next();
    });
    await call("7", "add", [2, 3], CTX);
    expect(result("7")).toBe(5);
    expect(seen).toEqual([["a", "add", [2, 3], SEEN, undefined], "b"]);
  });

  test("wraps JS plugin methods and reports the plugin name", async () => {
    registerPlugin({
      __tynd_plugin__: true,
      name: "mwp",
      methods: { double: (n: number) => n * 2 },
    });
    const seen: unknown[] = [];
    useMiddleware((ctx, next) => {
      seen.push([ctx.plugin, ctx.fn]);
      return next();
    });
    await call("8", "$plugin$mwp$double", [4]);
    expect(result("8")).toBe(8);
    expect(seen).toEqual([["mwp", "double"]]);
  });

  test("a throw rejects the call through the IPC error path", async () => {
    useMiddleware(() => {
      throw new Error("permission denied for add");
    });
    await call("9", "add", [1, 1]);
    expect(errorOf("9")).toContain("permission denied for add");
    expect(errorOf("9")).toContain("PERMISSION_DENIED");
  });

  test("middleware can rewrite arguments", async () => {
    useMiddleware((ctx, next) => {
      ctx.args = [10, 20];
      return next();
    });
    await call("10", "add", [1, 2]);
    expect(result("10")).toBe(30);
  });

  test("unknown functions still fail with UNKNOWN_FN through the chain", async () => {
    useMiddleware((_ctx, next) => next());
    await call("11", "nope");
    expect(errorOf("11")).toContain("UNKNOWN_FN");
  });
});

describe("reserved names", () => {
  test("a user export named __tynd_*__ is never dispatched", async () => {
    await call("20", "__tynd_admin__");
    expect(errorOf("20")).toContain("UNKNOWN_FN");
    expect(frames.some((f) => f.id === "20" && f.ok === true)).toBe(false);
  });

  test("the middleware chain does not rescue a reserved name", async () => {
    useMiddleware((_ctx, next) => next());
    await call("21", "__tynd_admin__");
    expect(errorOf("21")).toContain("UNKNOWN_FN");
  });

  test("__tynd_fe_emit__ still reaches the frontend-event dispatcher", async () => {
    await call("22", "__tynd_fe_emit__", ["evt", 1]);
    expect(result("22")).toBeNull();
  });
});

describe("shutdown", () => {
  test("a second close request reuses the first shutdown (beforeQuit runs once)", async () => {
    let runs = 0;
    registerPlugin(
      recordingPlugin("q", ["app:beforeQuit"], hookLog, {
        "app:beforeQuit": async () => {
          runs++;
          await Bun.sleep(20);
        },
      }),
    );
    const exit = spyOn(process, "exit").mockImplementation((() => undefined) as never);
    try {
      // `tynd:close` and the stdin `end` that follows both start a shutdown.
      await Promise.all([send({ type: "tynd:close" }), send({ type: "tynd:close" })]);
      await send({ type: "tynd:close" });
      expect(runs).toBe(1);
      expect(exit).toHaveBeenCalledTimes(1);
    } finally {
      exit.mockRestore();
    }
  });
});

describe("app:beforeStart", () => {
  test("runs the hooks and resolves", async () => {
    registerPlugin(recordingPlugin("s", ["app:beforeStart"], hookLog));
    await fireBeforeStartBounded({ name: "x" }, 1000);
    expect(hookLog.map((h) => h.name)).toEqual(["app:beforeStart"]);
  });

  test("gives up on a hung hook and warns instead of blocking startup", async () => {
    registerPlugin(
      recordingPlugin("hung", ["app:beforeStart"], hookLog, {
        "app:beforeStart": () => new Promise<void>(() => undefined),
      }),
    );
    const warn = spyOn(console, "warn").mockImplementation((() => undefined) as never);
    try {
      const started = Date.now();
      await fireBeforeStartBounded({}, 50);
      expect(Date.now() - started).toBeLessThan(1000);
      expect(String(warn.mock.calls[0]?.[0])).toContain("app:beforeStart");
    } finally {
      warn.mockRestore();
    }
  });
});

describe("plugin hooks over the wire", () => {
  test("ipc:before rejection stops the call", async () => {
    registerPlugin(
      recordingPlugin("guard", ["ipc:before"], hookLog, {
        "ipc:before": () => {
          throw new Error("blocked by guard");
        },
      }),
    );
    await call("12", "add", [1, 2]);
    expect(errorOf("12")).toContain("blocked by guard");
  });

  test("ipc:before/after see the command and the result", async () => {
    registerPlugin(recordingPlugin("spy", ["ipc:before", "ipc:after"], hookLog));
    await call("13", "add", [1, 2]);
    expect(result("13")).toBe(3);
    expect(hookLog).toEqual([
      { plugin: "spy", name: "ipc:before", args: ["add", [1, 2]] },
      { plugin: "spy", name: "ipc:after", args: ["add", 3] },
    ]);
  });

  test("tynd:window created/closed fire window hooks", async () => {
    registerPlugin(recordingPlugin("w", ["window:created", "window:closed"], hookLog));
    await send({ type: "tynd:window", event: "created", label: "settings" });
    await send({ type: "tynd:window", event: "closed", label: "settings" });
    expect(hookLog).toEqual([
      { plugin: "w", name: "window:created", args: [{ label: "settings" }] },
      { plugin: "w", name: "window:closed", args: [{ label: "settings" }] },
    ]);
  });

  test("tynd:ready fires app:ready plus user onReady handlers", async () => {
    registerPlugin(recordingPlugin("r", ["app:ready"], hookLog));
    let userReady = false;
    onReadyFns.push(() => {
      userReady = true;
    });
    await send({ type: "tynd:ready" });
    expect(hookLog.map((h) => h.name)).toEqual(["app:ready"]);
    expect(userReady).toBe(true);
  });

  test("tynd:close fires app:beforeQuit before onClose handlers, then exits", async () => {
    const order: string[] = [];
    registerPlugin(
      recordingPlugin("q", ["app:beforeQuit"], hookLog, {
        "app:beforeQuit": () => order.push("beforeQuit"),
      }),
    );
    onCloseFns.push(() => order.push("onClose"));
    const exit = spyOn(process, "exit").mockImplementation((() => undefined) as never);
    try {
      await send({ type: "tynd:close" });
      expect(order).toEqual(["beforeQuit", "onClose"]);
      expect(exit).toHaveBeenCalledWith(0);
    } finally {
      exit.mockRestore();
    }
  });
});

describe("cancelling an ordinary call", () => {
  test("aborts ctx.signal with an AbortError and drops the late result", async () => {
    const { aborts } = await import("./bun.fixture");
    aborts.length = 0;
    const running = call("abort-1", "waitForAbort", [], CTX);
    await Bun.sleep(5);
    await send({ type: "cancel", id: "abort-1" });
    await running;
    expect(aborts.length).toBe(1);
    const reason = aborts[0]?.reason as DOMException;
    expect(reason.name).toBe("AbortError");
    expect(frames.filter((f) => f.id === "abort-1")).toEqual([]);
  });

  test("a remote caller is not app content", async () => {
    await call("remote-1", "whoCalled", [], { ...CTX, origin: "https://remote.test" });
    expect(result("remote-1")).toEqual({
      ...SEEN,
      origin: "https://remote.test",
      isAppContent: false,
    });
  });
});

describe("host OS events", () => {
  test("a tynd:os_event frame reaches backend window handles", async () => {
    const { getWindow } = await import("../windows");
    const seen: unknown[] = [];
    const off = getWindow("main").onMoved((e) => seen.push(e));
    await send({
      type: "tynd:os_event",
      name: "window:moved",
      data: { label: "main", x: 1, y: 2 },
    });
    off();
    expect(seen).toEqual([{ label: "main", x: 1, y: 2 }]);
  });
});

describe("Channel arguments", () => {
  test("a placeholder becomes a channel that pushes to the calling webview only", async () => {
    const emitted: unknown[] = [];
    setEmitFn((name, payload, to) => emitted.push([name, payload, to]));
    await call("ch-1", "pushTwice", ["a", { __tynd_channel__: "c1" }], CTX);
    expect(result("ch-1")).toBe("closed");
    expect(emitted).toEqual([
      ["__tynd_ch__:c1", { m: "a-1" }, "child"],
      ["__tynd_ch__:c1", { m: "a-2" }, "child"],
      ["__tynd_ch__:c1", { end: true }, "child"],
    ]);
  });

  test("an object with extra keys is left alone", async () => {
    await call("ch-2", "pushTwice", ["a", { __tynd_channel__: "c1", x: 1 }], CTX);
    expect(errorOf("ch-2")).toContain("HANDLER_THREW");
  });
});

describe("cancel racing the stream registration", () => {
  test("a cancel that arrives while middleware is still running never starts the stream", async () => {
    let release: () => void = () => undefined;
    const gate = new Promise<void>((resolve) => {
      release = resolve;
    });
    useMiddleware(async (_ctx, next) => {
      await gate;
      return next();
    });
    const running = call("early", "countdown");
    await send({ type: "cancel", id: "early" });
    release();
    await running;
    await Bun.sleep(10);
    expect(frames.filter((f) => f.id === "early")).toEqual([]);
  });

  test("a late cancel for a finished call is forgotten", async () => {
    await call("done", "add", [1, 2]);
    await send({ type: "cancel", id: "done" });
    // Same id reused later must not be treated as pre-cancelled.
    await call("done", "countdown");
    expect(frames.filter((f) => f.id === "done" && f.type === "yield").length).toBe(2);
  });
});

interface HostFrame {
  id: string;
  api: string;
  method: string;
  args: unknown;
}

const g = globalThis as Record<string, unknown>;

describe("host trust handshake", () => {
  let sent: HostFrame[];

  beforeEach(() => {
    sent = [];
    setHostCallTransport((frame) => {
      sent.push(frame as HostFrame);
    });
  });
  afterEach(() => {
    for (const frame of sent) resolveHostCall(frame.id, true, null);
    setHostCallTransport(null);
    Reflect.deleteProperty(g, "__TYND_APP_INFO__");
    Reflect.deleteProperty(g, "__TYND_UPDATER_CONFIG__");
  });

  test("sends app.setInfo then updater.configure with the injected values", () => {
    const updater = {
      endpoints: ["https://e.example/m.json"],
      publicKeys: ["PUB"],
      channel: "stable",
    };
    g.__TYND_APP_INFO__ = { version: "2.3.4" };
    g.__TYND_UPDATER_CONFIG__ = updater;
    sendHostTrust();
    expect(sent.map((f) => [f.api, f.method])).toEqual([
      ["app", "setInfo"],
      ["updater", "configure"],
    ]);
    expect(sent[0]?.args).toEqual({ version: "2.3.4" });
    expect(sent[1]?.args).toEqual(updater);
  });

  test("forwards proxy and headers to updater.configure untouched", () => {
    const updater = {
      endpoints: ["https://e.example/m.json"],
      publicKeys: ["PUB"],
      proxy: "http://proxy.corp:3128",
      headers: { Authorization: "Bearer t" },
    };
    g.__TYND_UPDATER_CONFIG__ = updater;
    sendHostTrust();
    expect(sent[0]?.args).toEqual(updater);
  });

  test("skips updater.configure when the app has no updater block", () => {
    g.__TYND_APP_INFO__ = { version: "1.0.0" };
    sendHostTrust();
    expect(sent.map((f) => f.method)).toEqual(["setInfo"]);
  });

  test("sends nothing when the CLI injected nothing", () => {
    sendHostTrust();
    expect(sent).toEqual([]);
  });

  test("ignores a malformed version", () => {
    g.__TYND_APP_INFO__ = { version: 7 };
    sendHostTrust();
    expect(sent).toEqual([]);
  });
});

/** Read `count` newline-terminated JSON frames from a child's stdout. */
async function readFrames(stream: ReadableStream<Uint8Array>, count: number): Promise<Frame[]> {
  const decoder = new TextDecoder();
  const reader = stream.getReader();
  const out: Frame[] = [];
  let buf = "";
  while (out.length < count) {
    const { value, done } = await reader.read();
    if (done) break;
    buf += decoder.decode(value, { stream: true });
    const lines = buf.split("\n");
    buf = lines.pop() ?? "";
    for (const line of lines) if (line.trim()) out.push(JSON.parse(line) as Frame);
  }
  return out;
}

describe("boot frame order", () => {
  test("the first frame on stdout is tynd:config, host-trust calls follow", async () => {
    const fixture = new URL("./boot.fixture.ts", import.meta.url).pathname.replace(
      /^\/([A-Za-z]:)/,
      "$1",
    );
    const proc = Bun.spawn([process.execPath, fixture], {
      env: { ...process.env, TYND_ENTRY: ENTRY },
      stdin: "pipe",
      stdout: "pipe",
      stderr: "ignore",
    });
    try {
      const got = await readFrames(proc.stdout, 3);
      const shape = got.map((f) => {
        const h = f as Frame & { api?: string; method?: string };
        return f.type === "host_call" ? `${h.api}.${h.method}` : f.type;
      });
      expect(shape).toEqual(["tynd:config", "app.setInfo", "updater.configure"]);
    } finally {
      proc.kill();
    }
  }, 20_000);
});
